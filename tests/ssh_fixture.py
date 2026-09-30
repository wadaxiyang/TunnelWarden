"""Loopback-only SSH server for the Windows/Linux integration harness.

AsyncSSH owns the SSH server and channel/forwarding protocol. No production
code imports this fixture. Every process creates fresh, untrusted test keys.
"""
import asyncio
import json
import asyncssh


async def main():
    host_key = asyncssh.generate_private_key("ssh-ed25519")
    client_key = asyncssh.generate_private_key("ssh-ed25519")

    class Server(asyncssh.SSHServer):
        def begin_auth(self, username):
            return True

        def password_auth_supported(self):
            return True

        def validate_password(self, username, password):
            return username == "tester" and password == "fixture-only-password"

        def public_key_auth_supported(self):
            return True

        def validate_public_key(self, username, key):
            return username == "tester" and key == client_key.convert_to_public()

        def connection_requested(self, dest_host, dest_port, orig_host, orig_port):
            return dest_host == "127.0.0.1"

        def server_requested(self, listen_host, listen_port):
            return listen_host == "127.0.0.1"

    server = await asyncssh.create_server(
        Server, "127.0.0.1", 0, server_host_keys=[host_key],
        encoding=None,
    )
    print(json.dumps({
        "port": server.get_port(),
        "fingerprint": host_key.get_fingerprint(),
        "private_key": client_key.export_private_key().decode(),
        "encrypted_private_key": client_key.export_private_key(passphrase="fixture-key-passphrase").decode(),
    }), flush=True)
    await asyncio.Future()


asyncio.run(main())
