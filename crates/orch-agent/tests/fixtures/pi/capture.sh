#!/bin/sh
# Stands in for orch: `capture.sh hook|tap --agent pi --session <id>` appends the command and payload to $ORCH_CAPTURE.
{ printf '{"command":"%s","payload":' "$1"; cat; echo '}'; } >> "$ORCH_CAPTURE"
