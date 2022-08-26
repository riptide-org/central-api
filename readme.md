# Central API

The central api for the file-share-platform. Handles websocket conenctions from server agents, and requests from clients who wish to download a file.

## Instructions

```bash
git clone https://github.com/file-share-platform/central-api.git
cd central-api

cp .example.env .env
nano .env #update any configuration files

cargo run --release
```
