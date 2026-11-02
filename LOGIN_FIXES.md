# LibreFi LAN and Admin Login Fixes

This source update addresses the admin login failures seen when opening LibreFi at `http://10.0.0.1/admin` from a LAN-connected laptop.

## What changed

- LAN admin access is enabled by default. Set `LIBREFI_ADMIN_LAN=false` in the LibreFi process environment to restore the original LAN-side block.
- The login frontend uses `@noble/ciphers` AES-256-GCM rather than `crypto.subtle`. `crypto.subtle` is restricted to secure browser contexts and is normally unavailable on a plain-HTTP private IP address. The replacement preserves the existing Rust wire format: 12-byte nonce followed by AES-GCM ciphertext and tag, Base64 encoded.
- The login page now reports HTTP 401 (wrong key), HTTP 403 (LAN access policy), and other server/network failures separately.
- Frontend Base64 query payloads are URL-encoded.
- The QoS GET endpoint now uses the same encrypted admin-payload authentication as the other admin APIs; the Settings page sends its encrypted payload to that endpoint.

## Build the updated source

Build the React frontend first. This is mandatory because the Rust binary embeds the contents of the root `dist/` directory at compile time.

```sh
cd librefi-reactjs
npm install
npm run build
cd ..
cargo build --release
```

If you use pnpm instead of npm, run `pnpm install --no-frozen-lockfile` once to refresh `pnpm-lock.yaml`, then `pnpm build` and return to the project root for `cargo build --release`.

## Runtime configuration

LibreFi still requires `LIBREFI_KEY` at runtime. Use the same secret in the browser login form. The key must be at least 16 characters. Do not hard-code a production secret into the source tree or publish it in a repository.

LAN access is enabled without requiring `LIBREFI_ADMIN_LAN` to be set. Set it explicitly to `false` to disable LAN-side admin access again.

## Deployment note

The `/etc/init.d` script in the upstream README uses OpenWrt's `procd`. It will not work on a regular Debian/Ubuntu system without OpenWrt's service manager. Choose the startup service (OpenWrt procd or systemd) based on the mini PC's actual OS. The Rust binary itself is independent of that service-manager choice, but LibreFi's router-management commands still expect a compatible router environment.
