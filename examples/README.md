# screendoc examples

Runnable walkthroughs of the `screendoc` CLI. Each script is self-contained:
it records a short session in an isolated `$HOME`, renders the derived
artifacts, and verifies them. No state is left behind.

| Example | Shows |
|---|---|
| [`01-record-render.sh`](01-record-render.sh) | Record a command in wrapper mode, render GIF + MP4 from it |
| [`02-chapters-redact.sh`](02-chapters-redact.sh) | Manual + auto chapters, secret redaction, stills export |
| [`03-web-serve.sh`](03-web-serve.sh) | Serve a session to a browser with the embedded xterm.js player |
| [`04-encrypt.sh`](04-encrypt.sh) | Encrypt a session at rest (AES-256-GCM) and render from it |

## Running

```sh
# Build the release binary, then run everything:
./run-all.sh

# Or run a single example against any built binary:
./01-record-render.sh /path/to/screendoc
```

Requirements: `tmux`. The video artifacts (GIF/MP4/WebM/AV1) additionally
require `ffmpeg`; examples degrade to stills/cast export when it is missing.
`03-web-serve.sh` additionally requires `curl`.

The scripts set `HOME` to a temporary directory for the duration of the run,
so the session store (`~/.screendoc`) and the encryption key (`~/.screendoc/key`)
are created and discarded inside the sandbox. Nothing in your real home
directory is touched.

## What an example verifies

Each script ends with assertions (file existence, non-zero size, dimensions
where cheap to check) so that `./run-all.sh` doubles as an end-to-end smoke
test of the release pipeline's artifacts.
