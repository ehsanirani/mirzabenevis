#!/bin/sh
# Fake agent CLI: logs argv/stdin/PATH/cwd to $FAKE_LOG_DIR/<name>.log and answers.
# Behaviour via FAKE_MODE: ok | fail | hang | slowchild | noisy
name=$(basename "$0")
log=${FAKE_LOG_DIR:-/tmp}/$name.log
{
  echo "ARGC=$#"; i=0; for a in "$@"; do i=$((i+1)); printf 'ARG%d=[%s]\n' $i "$a"; done
  echo "PATH=$PATH"; echo "PWD=$(pwd)"
  echo "ENV_KEYS=$(env | cut -d= -f1 | sort | tr '\n' ' ')"
  echo "--- stdin"; cat; echo; echo "--- end"
} > "$log"
case "${FAKE_MODE:-ok}" in
  ok)
    if [ "$name" = claude ]; then
      echo '{"type":"system","subtype":"init"}'
      echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Fixed "}}}'
      echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"by claude."}}}'
      echo '{"type":"result","subtype":"success","is_error":false,"result":"Fixed by claude."}'
    else
      echo "Fixed by $name."
    fi ;;
  fail) echo "fatal: not logged in, run '$name login'" >&2; exit 7 ;;
  claudefail)
      echo '{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}'; exit 1 ;;
  hang) sleep 30 ;;
  slowchild) sleep 31 & echo "started"; wait ;;
esac
