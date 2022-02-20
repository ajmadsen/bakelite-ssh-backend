import sys
from typing import List
import paramiko
import tarfile
import argparse
import os


def parse_args(args: List[str]):
    parser = argparse.ArgumentParser(
        prog="bakelite-ssh-backend",
        description="Copies files from a .tar archive over a single SSH connection",
    )
    parser.add_argument("host", help="the host to copy to", metavar="HOST")
    parser.add_argument(
        "-t",
        "--tarfile",
        type=open,
        default=sys.stdin.buffer,
        help="optionally specify an existing .tar file to use",
    )
    parser.add_argument(
        "-C", "--chdir", help="change to this directory on the remote host"
    )
    parser.add_argument(
        "-p",
        "--port",
        help="the port on the host to connect to",
        default="22",
        type=int,
    )
    parser.add_argument(
        "-i",
        "--identity",
        help="the keyfile with which to identify ourselves to the server",
    )
    parser.add_argument(
        "-l", "--login-name", help="the user to use when connecting to the server"
    )
    return parser.parse_args(args)


def mkdir_r(client: paramiko.SFTPClient, path: str):
    if path == "" or path == ".":
        return
    try:
        client.stat(path)
    except FileNotFoundError:
        mkdir_r(client, os.path.dirname(path))
        client.mkdir(path)


def put_archive(tar_file: tarfile.TarFile, client: paramiko.SFTPClient):
    seen_dir = set()
    while True:
        info = tar_file.next()
        if info is None:
            return

        if info.isdir():
            client.mkdir(info.name, info.mode)
            client.utime(info.name, (info.mtime, info.mtime))

        if not info.isfile():
            print(f"skipping non-file {info.name}", file=sys.stderr)

        dirname = os.path.dirname(info.name)
        if not dirname in seen_dir:
            mkdir_r(client, dirname)
            seen_dir.add(dirname)

        src = tar_file.extractfile(info)
        client.putfo(src, info.name, info.size)
        client.chmod(info.name, info.mode)
        client.utime(info.name, (info.mtime, info.mtime))


def main(args: List[str] = None):
    if not args:
        args = sys.argv[1:]
    parsed = parse_args(args)

    login = os.getlogin()
    host = parsed.host
    if "@" in host:
        login, host = host.split("@", 1)

    if parsed.login_name is not None:
        login = parsed.login_name

    client = paramiko.SSHClient()
    client.load_system_host_keys()
    client.connect(host, port=parsed.port, key_filename=parsed.identity, username=login)

    sftp_client = client.open_sftp()
    if parsed.chdir is not None:
        try:
            sftp_client.stat(parsed.chdir)
        except FileNotFoundError:
            sftp_client.mkdir(parsed.chdir, 0o755)
        sftp_client.chdir(parsed.chdir)

    tar_file = tarfile.open(mode="r|*", fileobj=parsed.tarfile)

    try:
        put_archive(tar_file, sftp_client)
    finally:
        tar_file.close()
        parsed.tarfile.close()
        sftp_client.close()
        client.close()
