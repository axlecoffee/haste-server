<!-- SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee> -->
<!-- SPDX-License-Identifier: AGPL-3.0-only -->

## hst.axle.coffee

A public paste service with a native Rust API and a Leptos CSR WebAssembly
frontend. Trunk builds the browser assets. The runtime has no Node.js or Redis.
Pastes are kept indefinitely. Do not upload passwords or private information.

### Build and run

Requires Docker Engine with BuildKit and Docker Compose:

Copy `.env.example` to `.env` and fill in the credentials before starting.
The local `.env` is ignored by Git and excluded from the image and source archive.

```sh
docker compose config --quiet
docker compose build
# First deployment only: create/check the private bucket and test conditional writes.
docker compose run --rm haste --provision
docker compose up -d
```

Listens on `0.0.0.0:8292`, with no healthcheck. The runtime uses UID/GID
`1000:1000`, a read-only filesystem, CA certificates, and no capabilities.
Builds use Rust 1.99.0 on Alpine 3.24, Trunk 0.21.14, and `Cargo.lock`. Base distribution
packages still receive upstream updates. Hashed assets are served at `/assets/`.
Trunk's script template loads `boot.js` to work with the server CSP.
README and license notices are embedded at compile time; rebuild to update them.

### Runtime configuration

Compose loads `.env` into the container environment. Native runs load it with
dotenvy. Existing environment variables take precedence. The container is named
`hst-axlecoffee`; Compose manages the build image name.

| Setting | Value |
| --- | --- |
| RustFS keys | `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY` |
| Mongo key | `MONGODB_URI`, an SRV URI |
| S3 endpoint | `https://rustfs.axle.coffee` |
| Region | `ca-1` |
| Bucket | `haste-store`, private |

Mongo authentication and query options are preserved; the driver selects
database `hst-api`, collection `request_logs`. Credentials are copied once into
this project's `.env`, not mounted from another project's configuration.
Subsequent changes to those other files do not affect this service. Never print
credentials or pass them to builds. Recreate the container after changing `.env`.

`TRUSTED_PROXY_IPS` defaults to empty. Set only verified, exact tunnel socket
peer IPs, comma-separated. Untrusted clients cannot supply forwarding headers.
Packaging does not configure tunnels, DNS, cache rules, or external services.

### API and privacy

- `POST /documents`: UTF-8 text or a JSON string, at most 400000 request bytes;
  returns `201` with a `key`. Form and octet-stream bodies are raw UTF-8.
- `GET /documents/<key>`: JSON with `key` and `data`.
- `GET /raw/<key>`: plain text. Extensions are presentation hints.
- `/about`, `/about.md`, `/readme`, `/readme.md`, `/license`: built-in documents,
  also available through `/documents/` and `/raw/`.

Upload without stripping trailing newlines:

```sh
curl --fail-with-body -H 'Content-Type: text/plain; charset=utf-8' \
  --data-binary @example.txt http://localhost:8292/documents
```

Only `POST /documents` is limited to 120 requests per minute per client IP.
Creation responses carry rate-limit headers; rejection includes `Retry-After`.
GET is unlimited. The limiter is process-local, not shared across replicas.

Paste reads use `public, max-age=300, s-maxage=86400`; errors and POST use
`no-store`; built-in documents use `no-cache`. Hashed assets are immutable for
one year. Edge cache eligibility is separate configuration, not a guarantee.

Origin logs record method, path, status, timestamp, duration, client and peer
IP, user agent, and available trusted location headers. They exclude bodies,
cookies, and authorization headers. Pastes and logs have no automatic expiry.
Location is approximate. Edge hits do not reach the origin and are not logged.
Mongo failures and queue overflow can drop logs; drops are reported while the
paste service remains available.

### Source and licenses

New work: Copyright 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>,
**AGPL-3.0-only**. See [the license](/raw/license) and `THIRD_PARTY_NOTICES`.

Images built by this Dockerfile offer their corresponding source without
authentication or charge at [/source.tar.gz](/source.tar.gz). This includes
application source, Cargo.lock, vendored Rust dependencies and notices, and
build/deployment instructions. Extract it and follow the build commands above.
Keep this route available to network users of modified deployments. No new
repository URL is claimed: the existing Git remote is the Skyra upstream.

Credit to **John Crepezzi** for the original Haste interface and protocol,
**Skyra Project** for the previous MIT-licensed implementation, and
**Catppuccin** for the MIT-licensed Mocha palette. Syntax/theme notices are at
[/syntax-notices.txt](/syntax-notices.txt). Dependency license files are in the
source archive's `vendor/` tree.

The Cargo workspace contains `server/` for the native API and `web/` for the
WASM frontend. Each has its own `Cargo.toml` and `src/`. The root `Cargo.lock`
locks dependencies for both. No Node, Yarn, or TypeScript implementation remains.

### Validation

```sh
reuse lint
cargo check --locked --workspace
cargo test --locked -p haste-server
docker compose config --quiet
docker build --check .
```

CI builds locally without publishing to an upstream registry. A full image
build and browser smoke test are required before deployment.
