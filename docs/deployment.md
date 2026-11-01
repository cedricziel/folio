# Deploy your own Folio

Folio is one binary with its web assets built in. Run it as a container or as
a release binary. Out of the box it keeps documents on local disk and its data
in SQLite, so you need nothing else to start.

## Run the container

The image is `ghcr.io/cedricziel/folio`. Pick a tag:

| Tag                        | Contents                                     |
| -------------------------- | -------------------------------------------- |
| `edge`                     | The latest commit on `main`                  |
| `0.1.0` and other versions | One release                                  |
| `latest`                   | The newest release that is not a pre-release |

The container listens on port 3000, runs as uid 10001 and keeps everything in
`/data`: the SQLite database (`/data/folio.db`) and the documents
(`/data/documents`).

```sh
docker run -d --name folio \
  -p 3000:3000 \
  -v folio-data:/data \
  -e FOLIO_SERVER__PUBLIC_URL=https://folio.example.com \
  ghcr.io/cedricziel/folio:0.1.0
```

Set `FOLIO_SERVER__PUBLIC_URL` to the address people type into their browser.
The container listens on `0.0.0.0`, and with authentication on Folio refuses to
start there without a public URL. See [Public URL](auth.md#public-url).

If you mount a host directory instead of a named volume, make it writable by
uid 10001.

The same setup with Docker Compose:

```yaml
services:
  folio:
    image: ghcr.io/cedricziel/folio:0.1.0
    restart: unless-stopped
    ports:
      - "3000:3000"
    environment:
      FOLIO_SERVER__PUBLIC_URL: https://folio.example.com
    volumes:
      - folio-data:/data

volumes:
  folio-data:
```

Every setting is a `FOLIO_*` environment variable; see
[configuration.md](configuration.md). AI providers and their keys can also be
set later on the `/settings` page.

## Sign in for the first time

On a fresh install Folio prints a one-time setup link to its log:

```sh
docker logs folio
```

```
No owner account exists yet. Open this link to create it:

  https://folio.example.com/setup?token=...
```

Open it and create the owner account. The token changes on every start until
an account exists. See [First run](auth.md#first-run).

With OpenID Connect and `allow_local_login = false`, there is no setup link:
the first person to sign in through the provider becomes the owner. See
[OpenID Connect](auth.md#openid-connect).

## Run the release binary

[GitHub Releases](https://github.com/cedricziel/folio/releases) has archives
for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and
`aarch64-apple-darwin`, each with a `.sha256` file. An archive holds
`folio-app`, a `config.toml` and the license.

```sh
tar -xzf folio-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd folio-v0.1.0-x86_64-unknown-linux-gnu
./folio-app
```

Folio reads `./config.toml` (or the file in `FOLIO_CONFIG`). The bundled one
listens on `127.0.0.1:3000`, writes `folio.db` and `./data` in the working
directory, and needs no public URL on loopback. To serve other machines, set
`server.host` and `server.public_url`.

The container ships the PDF and OCR tools Folio calls (`pdftoppm`,
`pdfinfo`, `pdftotext`, ImageMagick's `convert` and `tesseract`). With the
binary, install them yourself.

## Put it behind a reverse proxy

Terminate TLS at the proxy and forward to port 3000. Folio never reads `Host`
or `X-Forwarded-*`, so `public_url` must be exactly the URL clients use,
scheme included. A mismatch breaks sign-in and OAuth.

To serve MCP through the proxy, list the public hostname in
`mcp.allowed_hosts`. See [mcp.md](mcp.md#configuration).

If the proxy puts its own single sign-on in front of Folio, let these paths
through without it:

- `/.well-known/*`
- `/oauth/token`, `/oauth/revoke`, `/oauth/register`
- `/api/*`
- `/mcp` and `/mcp/*`

The Apple apps, Shortcuts and MCP clients sign in with Folio's own OAuth and
send bearer tokens on these paths; they cannot complete a browser login at the
proxy. Leave `/oauth/authorize` behind the proxy: it is the page a person sees.

To let the proxy's login create Folio sessions too, see
[Forward auth](auth.md#forward-auth).

## Next steps: Postgres and S3

- Use PostgreSQL instead of SQLite: [database.md](database.md#postgresql).
- Keep documents in S3 or an S3-compatible service: [storage.md](storage.md#s3-and-compatible-services).
- [`docker-compose.example.yml`](../docker-compose.example.yml) runs Folio with
  PostgreSQL and MinIO.
- Export traces, metrics and logs with OpenTelemetry: [observability.md](observability.md).

## Upgrade

Pull the new image or unpack the new release, then restart. Folio runs its
database migrations at startup. Back up first.

```sh
docker compose pull
docker compose up -d
```

## Back up

What to back up depends on where Folio keeps its data:

- **Defaults (SQLite and local storage):** the `/data` volume, or for the
  binary the database file and the storage directory.
- **PostgreSQL and S3:** a database dump and the bucket.

Back up the database and the documents together, so a restore does not point
at files that are missing. Stop Folio while copying an SQLite file, or copy it
with `sqlite3 folio.db ".backup backup.db"`.
