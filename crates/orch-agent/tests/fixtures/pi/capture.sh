#!/bin/sh
# Stands in for orch: `capture.sh hook --agent pi --session <id>` appends the payload to $ORCH_CAPTURE.
{ cat; echo; } >> "$ORCH_CAPTURE"
