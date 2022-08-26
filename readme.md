# Central API

![Test](https://github.com/riptide-org/central-api/actions/workflows/precommit.yml/badge.svg)
[![codecov](https://codecov.io/gh/riptide-org/central-api/branch/main/graph/badge.svg?token=ALQI2M77DH)](https://codecov.io/gh/riptide-org/central-api)

The central api for the file-share-platform. Handles websocket connections from server agents, and requests from clients who wish to download a file.

## Instructions

```bash
git clone https://github.com/file-share-platform/central-api.git
cd central-api

cp .example.env .env
nano .env #update any configuration files

cargo run --release
```
