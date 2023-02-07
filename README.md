# Country counter

A demo project for integrating Cloudflare Workers and [libsql-client](https://docs.rs/libsql-client/latest/libsql_client/).

## Description

This demo project implements a simple per-location counter. Each visit of the page bumps
a counter associated with the location of the Cloudflare Worker instance that ran
the particular request.

All visited locations are also visualized with the help of [Mappa](https://mappa.js.org/docs/simple-map.html).

## Setup

To prepare the environment, set up Cloudflare Workers' `wrangler` tool:
https://developers.cloudflare.com/workers/wrangler/install-and-update/

To set up the database, first join the beta for ChiselStrike Turso: https://chiselstrike.com/
Then, create your database and create the following entries in `.dev.vars` file, or register them
as [secrets](https://developers.cloudflare.com/workers/wrangler/commands/#secret) on Cloudflare.
```
LIBSQL_CLIENT_URL = "https://<YOUR-DB-URL-HERE>"
LIBSQL_CLIENT_USER = "<YOUR-USERNAME-HERE>"
LIBSQL_CLIENT_PASS = "<YOUR-PASS-HERE>"
```

## Development

To run the example:
1. Run `wrangler dev`
2. Visit your page at localhost:8787

## Live demo

The example is also deployed live here: https://country-counter.p-sarna.workers.dev/
