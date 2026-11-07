#!/usr/bin/env python3
"""最小 SOCKS5 / HTTP CONNECT 代理，仅供 E2E 测试使用。

每个成功转发的目标写一行到 --log，测试脚本据此证明流量确实经过代理。
凭据可选：给了 --user 就要求认证（SOCKS5 user/pass、HTTP Basic）。
"""

import argparse
import asyncio
import base64


def log_line(path: str, text: str) -> None:
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(text + "\n")


async def pipe(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.IncompleteReadError):
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass


async def relay(local_reader, local_writer, host: str, port: int, log: str) -> None:
    try:
        remote_reader, remote_writer = await asyncio.open_connection(host, port)
    except OSError as error:
        log_line(log, f"dial-failed {host}:{port} {error}")
        local_writer.close()
        return
    log_line(log, f"relay {host}:{port}")
    await asyncio.gather(
        pipe(local_reader, remote_writer),
        pipe(remote_reader, local_writer),
    )


async def handle_socks(reader, writer, args) -> None:
    try:
        greeting = await reader.readexactly(2)
        if greeting[0] != 5:
            writer.close()
            return
        methods = await reader.readexactly(greeting[1])
        want_auth = bool(args.user)
        if want_auth:
            if 2 not in methods:
                writer.write(b"\x05\xff")
                await writer.drain()
                writer.close()
                return
            writer.write(b"\x05\x02")
            await writer.drain()
            version = await reader.readexactly(1)
            if version != b"\x01":
                writer.close()
                return
            user_len = (await reader.readexactly(1))[0]
            user = (await reader.readexactly(user_len)).decode()
            pass_len = (await reader.readexactly(1))[0]
            password = (await reader.readexactly(pass_len)).decode()
            ok = user == args.user and password == (args.password or "")
            writer.write(b"\x01\x00" if ok else b"\x01\x01")
            await writer.drain()
            if not ok:
                writer.close()
                return
        else:
            if 0 not in methods:
                writer.write(b"\x05\xff")
                await writer.drain()
                writer.close()
                return
            writer.write(b"\x05\x00")
            await writer.drain()

        head = await reader.readexactly(4)
        if head[0] != 5 or head[1] != 1:
            writer.close()
            return
        atyp = head[3]
        if atyp == 1:
            host = ".".join(str(b) for b in await reader.readexactly(4))
        elif atyp == 3:
            length = (await reader.readexactly(1))[0]
            host = (await reader.readexactly(length)).decode()
        elif atyp == 4:
            raw = await reader.readexactly(16)
            host = ":".join(
                f"{raw[i]:02x}{raw[i + 1]:02x}" for i in range(0, 16, 2)
            )
        else:
            writer.close()
            return
        port = int.from_bytes(await reader.readexactly(2), "big")
        writer.write(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
        await writer.drain()
        await relay(reader, writer, host, port, args.log)
    except (asyncio.IncompleteReadError, ConnectionError):
        writer.close()


async def handle_http(reader, writer, args) -> None:
    try:
        head = await reader.readuntil(b"\r\n\r\n")
    except (asyncio.IncompleteReadError, ConnectionError):
        writer.close()
        return
    lines = head.decode("latin-1").split("\r\n")
    parts = lines[0].split()
    if len(parts) < 2 or parts[0] != "CONNECT":
        writer.write(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
        await writer.drain()
        writer.close()
        return
    if args.user:
        expected = base64.b64encode(
            f"{args.user}:{args.password or ''}".encode()
        ).decode()
        auth = next(
            (
                line.split(":", 1)[1].strip()
                for line in lines[1:]
                if line.lower().startswith("proxy-authorization:")
            ),
            "",
        )
        if auth != f"Basic {expected}":
            writer.write(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
            await writer.drain()
            writer.close()
            return
    host, _, port = parts[1].rpartition(":")
    if not host:
        host, port = parts[1], "443"
    try:
        port = int(port)
    except ValueError:
        writer.write(b"HTTP/1.1 400 Bad Request\r\n\r\n")
        await writer.drain()
        writer.close()
        return
    writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
    await writer.drain()
    await relay(reader, writer, host, port, args.log)


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kind", choices=["socks5", "http"], required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--user", default="")
    parser.add_argument("--password", default="")
    args = parser.parse_args()
    handler = handle_socks if args.kind == "socks5" else handle_http
    server = await asyncio.start_server(
        lambda r, w: handler(r, w, args), "127.0.0.1", args.port
    )
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
