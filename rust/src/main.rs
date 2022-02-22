#![feature(trait_alias)]

use std::collections::BTreeSet;
use std::io::{Error, ErrorKind};
use std::path::Path;
use std::sync::Arc;

use async_compat::CompatExt;
use async_io::Async;
use async_ssh2_lite::AsyncSession;
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
    BufReader::new(Box::new(r))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let reader = match args.tarfile {
        Some(f) => wrap_readable(File::open(f).await?),
        None => wrap_readable(tio::stdin()),
    };
    let archive = Archive::new(reader.compat());

    let (login, host) = match args.host.split_once('@') {
        Some(x) => x,
        None => (args.login.as_str(), args.host.as_str()),
    };

    let sock = TcpStream::connect((host, args.port)).await?;
    let sock = Async::new(sock.into_std()?)?;
    let mut session = AsyncSession::new(sock, None)?;

    session.handshake().await?;
    session.userauth_agent_with_try_next(login).await?;
    let sftp = session.sftp().await?;

    println!("connected!");

    let _os_base_path = std::path::PathBuf::from(args.chdir.clone().unwrap_or(".".to_owned()));
    let base_path = SimplePath::new(args.chdir.unwrap_or(".".to_owned()));
    let seen_paths = Arc::new(RwLock::new(BTreeSet::<String>::new()));

    archive
        .entries()?
        .try_for_each(|mut ent| {
            let base_path = &base_path;
            let seen_paths = seen_paths.clone();
            let sftp = &sftp;
            let session = &session;
            async move {
                if !ent.header().entry_type().is_file() {
                    return Ok(());
                }
                let dst = base_path.join(ent.path()?.to_string_lossy());
                let ancestors: Vec<_> = dst
                    .ancestors()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .filter(|&p| !p.is_empty())
                    .collect();
                println!("ancestors: {:?}", ancestors);
                for pth in ancestors {
                    if pth.is_empty() || seen_paths.read().await.contains(pth) {
                        continue;
                    }
                    let npth = Path::new(pth);
                    match sftp.stat(npth.as_ref()).await {
                        Ok(_) => (),
                        Err(_) => {
                            // println!("mkdir {}", pth);
                            sftp.mkdir(npth.as_ref(), 0o755).await?
                        }
                    }
                    {
                        let mut seen_paths = seen_paths.write().await;
                        seen_paths.insert(pth.to_owned());
                    }
                }

                let sz = ent.header().size()?;
                // println!("put {} [{} bytes]", dst.as_str(), sz);

                // let mut fp = sftp.open_mode(OsString::from(&dst).as_ref(), OpenFlags::TRUNCATE | OpenFlags::WRITE, 0o644, OpenType::File).await
                //     .map_err(|e| Error::new(ErrorKind::Other, format!("could not open file: {:?}", e)))?;
                // let bytes = fio::copy(&mut ent, &mut fp).await
                //     .map_err(|e| Error::new(ErrorKind::Other, format!("could not write bytes: {:?}", e)))?;
                // fp.close().await?;
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
