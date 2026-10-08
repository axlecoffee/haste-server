<!-- SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee> -->
<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# haste-server / hst.axle.coffee

Yet another haste/paste service, this time, in Rust! (along with a Leptos CSR WebAssembly frontend)

Pastes are stored indefinitely, so be mindful of what you upload!

## Quick Contact
- abuse@axle.coffee (abuse/illegal/malicious/urgent content)
- dmca@axle.coffee (DMCA takedown requests)
- content@axle.coffee (other/personal content removal requests)
- contact@axle.coffee (inquiries/help/mass-retrieval/data-deletion)
- axle.coffee (on discord)
- GitHub issues (wherein appropriate)

### Build and run

Docker Engine with BuildKit and Docker Compose. Copy `.env.example` to `.env`
and fill in the credentials, obviously.

```sh
docker compose config --quiet
docker compose build
# First deployment only: create/check the private bucket and test conditional writes.
docker compose run --rm haste --provision
docker compose up -d
```

Listens on `0.0.0.0:8292`. (there is no healthcheck, pr if u want) The container is named
`hst-axlecoffee` and runs as UID/GID `1000:1000` with a read-only filesystem,
CA certificates, and no capabilities

Builds use Rust 1.99.0 on Alpine 3.24 with Trunk 0.21.14 and `Cargo.lock`.
Note that basedistribution packages still receive upstream updates. 

README and license notices are embedded at compile time, rebuild to update them.

### Runtime configuration

| Setting     | Value                                    |
| ----------- | ---------------------------------------- |
| RustFS keys | `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY` |
| Mongo key   | `MONGODB_URI`, an (in my case)SRV URI    |
| S3 endpoint | `an s3 endpoint url` (rustFS!)           |
| Region      | `ca-1`                                   |
| Bucket      | `haste-store`, (should be private)       |


`TRUSTED_PROXY_IPS` is empty by default. if you need it, set it to a peer IP or comma seperated list of peers e.g. in the case of HAProxy, nginx, cross-server proxies, local nets and intranets, etc!

### API

- `POST /documents`: UTF-8 text or a JSON string, at most 400000 request bytes;
  returns `201` with a `key`. Form and octet-stream bodies are raw UTF-8.
- `GET /documents/<key>`: JSON with `key` and `data`.
- `GET /raw/<key>`: plain text. Extensions are presentation stuff
- `/about`, `/about.md`, `/readme`, `/readme.md`, `/license`: reserved routes which are also available through `/documents/` and `/raw/`.

Upload without stripping trailing newlines:

```sh
curl --fail-with-body -H 'Content-Type: text/plain; charset=utf-8' \
  --data-binary @example.txt http://localhost:8292/documents
```

`POST /documents` is, on my setup, ratelimited to 120 RPM (requests per minute), per IP.
This is to prevent abuse, obviously

### Logs and retention

We retain logs for anti-abuse and operational/load balancing purposes.

Origin logs include record method, path, status, timestamp, duration, client and peer IP, UA (User Agent), and whatever trusted location headers arrive.
We do not log request bodies, cookies, or authorization headers.
Pastes and logs have no automatic expiry, however, they may be removed manually or due to storage constraints.

### Abuse

You and your IP are responsible for any abuse originating from your connection. Misuse of the service will result in appropriate action - including but not limited to IP bans, rate limiting, and legal action wherein applicable.

Content posted must comply with both your (uploader)'s local laws and the service's local laws. Failure to do so may result in the actions described above.

Obviously, if any content violates these rules, it should be reported to abuse@axle.coffee or on discord @axle.coffee or via a GitHub issue, if appropriate.

Content uploaded should not be considered private or confidential. Users should avoid uploading sensitive information, as the host retains full, persistent access to all uploaded content.

### Takedown Policy

If you rightly believe that content uploaded violates a License or other applicable law, or you wish to request its removal for any reason, please contact dmca@axle.coffee for DMCA takedown requests, and content@axle.coffee for other content removal requests. (or @axle.coffee on Discord, GitHub issues where appropriate.)

### Source and licenses

(my) Work: Copyright 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>,
AGPL-3.0-only. See [the license](/raw/license) and `THIRD_PARTY_NOTICES`.

Images built by this Dockerfile offer their corresponding source without
authentication or charge at [/source.tar.gz](/source.tar.gz). That includes
application source, `Cargo.lock`, vendored Rust dependencies and notices, and
build/deployment instructions. Extract it and run the build commands above.

Credits to John Crepezzi for the original Haste interface and protocol, the
Skyra Project for the previous MIT-licensed implementation, and Catppuccin for
the MIT-licensed Mocha(etc.) palette. Syntax and theme notices are at
[/syntax-notices.txt](/syntax-notices.txt). Dependency license files live in the
source archive's `vendor/` tree~

### Validation

```sh
reuse lint
cargo fmt --all -- --check
cargo clippy --locked --workspace -- -D warnings
cargo check --locked --workspace
cargo test --locked --workspace
docker compose config --quiet
docker build --check .
```

