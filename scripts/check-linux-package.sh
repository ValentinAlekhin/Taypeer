#!/bin/sh
# Verify the installed production entry point rather than a test worker binary.
set -eu
package=${1:-result}
[ -x "$package/bin/taypeer" ]
[ -x "$package/bin/taypeer-cli" ]
[ ! -e "$package/bin/taypeer-ui-worker" ]
[ -f "$package/share/applications/io.taypeer.Taypeer.desktop" ]
# The installed CLI must start without acquiring credentials.
"$package/bin/taypeer-cli" --help >/dev/null
# The production worker rejects malformed private IPC.
worker_status=0
printf '%s\n' 'PUBLIC malformed worker frame' | "$package/bin/taypeer" __worker >/dev/null 2>&1 || worker_status=$?
if [ "$worker_status" -ne 1 ]; then
    echo "Packaged worker did not reject malformed IPC through its runtime" >&2
    exit 1
fi
smoke_status=0
"$package/bin/taypeer" --smoke-test >/dev/null 2>&1 || smoke_status=$?
if [ "$smoke_status" -ne 2 ]; then
    echo "Production package did not reject the public demo launch mode" >&2
    exit 1
fi
echo "Linux package: entry points, desktop integration and production worker verified"
