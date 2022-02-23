#![feature(trait_alias)]

use std::collections::BTreeSet;
use std::io::{Error, ErrorKind};
use std::path::Path;
use std::sync::Arc;

use async_compat::CompatExt;
use async_io::Async;
use async_ssh2_lite::{AsyncSession, AsyncSftp};
use async_tar::Archive;
use clap::Parser;
use futures::{io as fio, prelude::*};
use tokio::{
    fs::File,
    io::{self as tio, BufReader},
    net::TcpStream,
    sync::RwLock,
};

use bakelite_ssh_backend::SimplePath;

trait Readable = tio::AsyncRead + Unpin + Send + Sync;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The tarfile to read from instead of stdin
    #[clap(short, long)]
    tarfile: Option<String>,

    /// The port to connect to the server on
    #[clap(short, long, default_value_t = 22)]
    port: u16,

    /// The username to connect with
    #[clap(short, long, default_value_t = whoami::username())]
    login: String,

    /// The private key to authenticate with
    #[clap(short, long)]
    identity: Option<String>,

    /// The directory to change to upon login
    #[clap(short = 'C', long)]
    chdir: Option<String>,

    /// The host to connect to, can also be specified as user@HOST
    host: String,
}

fn wrap_readable<'a>(r: impl Readable + 'a) -> BufReader<Box<dyn Readable + 'a>> {
    BufReader::with_capacity(8 * 1024, Box::new(r))
}

async fn connect_from_args(
    args: &Args,
) -> Result<AsyncSession<std::net::TcpStream>, Box<dyn std::error::Error>> {
    let (login, host) = match args.host.split_once('@') {
        Some(x) => x,
        None => (args.login.as_str(), args.host.as_str()),
    };

    let sock = TcpStream::connect((host, args.port)).await?;
    let sock = Async::new(sock.into_std()?)?;
    let mut session = AsyncSession::new(sock, None)?;

    session.handshake().await?;
    session.userauth_agent_with_try_next(login).await?;
    Ok(session)
}

async fn mkdir_r<T, P: Into<SimplePath>>(
    sftp: &AsyncSftp<T>,
    pth: P,
    seen_paths: Arc<RwLock<BTreeSet<String>>>,
) -> Result<(), std::io::Error> {
    let pth = pth.into();
    let ancestors: Vec<_> = pth
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter(|&p| !p.is_empty())
        .collect();
    // println!("ancestors: {:?}", ancestors);
    for pth in ancestors {
        if pth.is_empty() || seen_paths.read().await.contains(pth) {
            continue;
        }
        let npth = Path::new(pth);
        match sftp.stat(npth).await {
            Ok(_) => (),
            Err(_) => {
                // println!("mkdir {}", pth);
                sftp.mkdir(npth, 0o755).await?
            }
        }
        {
            let mut seen_paths = seen_paths.write().await;
            seen_paths.insert(pth.to_owned());
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let reader = match args.tarfile.as_ref() {
        Some(f) => wrap_readable(File::open(f).await?),
        None => wrap_readable(tio::stdin()),
    };
    let archive = Archive::new(reader.compat());

    let session = connect_from_args(&args).await?;
    let sftp = Arc::new(session.sftp().await?);

    println!("connected!");

    let base_path = SimplePath::new(args.chdir.unwrap_or(".".to_owned()));
    let seen_paths = Arc::new(RwLock::new(BTreeSet::<String>::new()));

    let tmp_path = base_path.join(".tmp");
    mkdir_r(&sftp, tmp_path.as_str(), seen_paths.clone()).await?;

    archive
        .entries()?
        .try_for_each(|mut ent| {
            let base_path = &base_path;
            let seen_paths = seen_paths.clone();
            let sftp = sftp.clone();
            let session = &session;
            async move {
                if !ent.header().entry_type().is_file() {
                    return Ok(());
                }
                let dst = base_path.join(ent.path()?.to_string_lossy());
                mkdir_r(&sftp, dst.ancestors().skip(1).next().unwrap(), seen_paths).await?;

                let sz = ent.header().size()?;
                println!("put {} [{} bytes]", dst.as_str(), sz);

                let mut ch = session
                    .scp_send(Path::new(dst.as_str()), 0o644, sz, None)
                    .await
                    .map_err(|e| {
                        Error::new(ErrorKind::Other, format!("could not open file: {:?}", e))
                    })?;
                let bytes = fio::copy(&mut ent, &mut ch).await.map_err(|e| {
                    Error::new(ErrorKind::Other, format!("could not write bytes: {:?}", e))
                })?;
                ch.close().await?;

                if bytes == sz {
                    Ok(())
                } else {
                    Err(Error::new(
                        ErrorKind::Other,
                        format!("expected {} bytes but only wrote {}", sz, bytes),
                    ))
                }
            }
        })
        .await?;

    session.disconnect(None, "goodbye", None).await?;

    Ok(())
}
