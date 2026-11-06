#!/usr/bin/env bash
# End-to-end checks for mirzabenevis-cli backend detection and transports.
# Never calls a real AI service: mocks on 127.0.0.1, fake agent CLIs, and a
# guard `claude` that shadows the real one.
#   cargo build -p mirzabenevis-cli && tests-e2e/run_all.sh
# Checks named "BUG:" are expected to FAIL until the defect is fixed.
set -u
cd "$(dirname "$0")"
. ./lib.sh
PORT=${PORT:-18531}
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "PASS  $1"; }
bad() { fail=$((fail+1)); echo "FAIL  $1"; [ -n "${2:-}" ] && echo "      $2"; }
expect() { # name, actual, expected-substring
  case "$2" in *"$3"*) ok "$1";; *) bad "$1" "got: $(printf %s "$2" | head -c 300)";; esac; }

LOG=$T/mock.log; : > "$LOG"
python3 mock_server.py $PORT "$LOG" 2>/dev/null & MAIN=$!
MOCK_TAGS=bge python3 mock_server.py $((PORT+1)) "$T/tags1.log" & BGE=$!
MOCK_TAGS=401 python3 mock_server.py $((PORT+2)) "$T/tags2.log" & U401=$!
MOCK_TAGS=embedonly python3 mock_server.py $((PORT+3)) "$T/tags3.log" & EMB=$!
trap 'kill $MAIN $BGE $U401 $EMB 2>/dev/null; pkill -P $$ sleep 2>/dev/null' EXIT
sleep 1
MZ_ENV="SHELL=/nonexistent OLLAMA_HOST=127.0.0.1:$((PORT+3))"

oai_cfg() { writecfg <<EOF
[provider]
use = "custom"
timeout_secs = ${2:-10}
[provider.custom]
kind = "openai"
base_url = "http://127.0.0.1:$PORT/$1/v1"
model = "mock-model"
api_key_env = "MYKEY"
EOF
}
ant_cfg() { writecfg <<EOF
[provider]
use = "custom"
[provider.custom]
kind = "anthropic"
base_url = "http://127.0.0.1:$PORT/$1"
model = "claude-x"
api_key = "ak-123"
EOF
}
run() { echo "this are a test" | MZ_ENV="$MZ_ENV MYKEY=sk-test" mz run fix --print 2>&1; echo "exit=$?"; }
lastreq() { tail -1 "$LOG"; }

echo "--- OpenAI-compatible SSE"
oai_cfg ok;              out=$(run); expect "openai basic stream" "$out" $'Hello, world.\nexit=0'
r=$(lastreq)
expect "openai Bearer header" "$r" '"authorization": "Bearer sk-test"'
expect "openai sends max_tokens" "$r" '"max_tokens": 16384'
expect "openai temperature for custom" "$r" '"temperature": 0.2'
case "$r" in *0.2000000*) bad "temperature sent as clean 0.2";; *) ok "temperature sent as clean 0.2";; esac
oai_cfg comments;        expect "comments, role-only and reasoning deltas skipped" "$(run)" $'AB\nexit=0'
oai_cfg midstream_error; expect "error chunk inside 200 stream fails" "$(run)" "upstream rate limited"
oai_cfg utf8split;       expect "UTF-8 split byte-by-byte" "$(run)" "سلام دنیا ✓ 😀"
oai_cfg crlf;            expect "CRLF line endings" "$(run)" $'x\nexit=0'
oai_cfg http500;         expect "HTTP 500 body surfaced" "$(run)" "internal kaboom"
oai_cfg fenced;          expect "code fences stripped" "$(run)" $'fixed text\nexit=0'
oai_cfg nolf;            expect "last data line without trailing newline kept" "$(run)" "first last"
oai_cfg empty;           expect "empty answer is an error, not exit 0" "$(run)" "exit=1"
oai_cfg nonstream;       expect "non-SSE JSON answer is not silently empty" "$(run)" "full answer"
oai_cfg length;          expect "finish_reason=length reported (truncated text)" "$(run)" "exit=1"
oai_cfg slow 2
s=$(date +%s); out=$(run); took=$(( $(date +%s) - s ))
expect "slow server hits timeout_secs" "$out" "exit=1"
[ $took -le 4 ] && ok "timeout fires in ~2s ($took s)" || bad "timeout took $took s"
expect "timeout message is the friendly one" "$out" "no response for 2s"
oai_cfg huge
echo x | MZ_ENV="$MZ_ENV" mz run fix --print 2>"$T/pipe.err" | head -1 >/dev/null
if grep -q panicked "$T/pipe.err"; then bad "no panic when stdout is closed (| head)" "$(grep -m1 'failed printing' "$T/pipe.err")"; else ok "no panic when stdout is closed (| head)"; fi

echo "--- Anthropic SSE"
ant_cfg ok;       expect "anthropic basic" "$(run)" $'Fixed text.\nexit=0'
r=$(lastreq)
expect "anthropic x-api-key" "$r" '"x-api-key": "ak-123"'
expect "anthropic-version header" "$r" '"anthropic-version": "2023-06-01"'
case "$r" in *temperature*) bad "anthropic: no temperature" "$r";; *) ok "anthropic: no temperature";; esac
expect "anthropic thinking disabled" "$r" '"thinking": {"type": "disabled"}'
ant_cfg thinking; out=$(run); expect "thinking blocks skipped" "$out" $'Fixed text.\nexit=0'
case "$out" in *SECRET*) bad "thinking text leaked";; *) ok "thinking text not leaked";; esac
ant_cfg error;    expect "error event fails" "$(run)" "Overloaded"
ant_cfg http401;  expect "401 surfaced" "$(run)" "invalid x-api-key"
ant_cfg maxtokens; expect "stop_reason=max_tokens reported" "$(run)" "exit=1"
ant_cfg ok
out=$(echo hi | MZ_ENV="$MZ_ENV http_proxy=http://127.0.0.1:9 HTTP_PROXY=http://127.0.0.1:9" mz run fix --print 2>&1)
expect "anthropic custom on 127.0.0.1 bypasses HTTP proxy" "$out" "Fixed text."

echo "--- local servers (via OLLAMA_HOST)"
j() { MZ_ENV="$1" mz providers --json; }
out=$(j "SHELL=/nonexistent OLLAMA_HOST=127.0.0.1:$((PORT+3))")
expect "ollama with only embedding models -> missing" "$out" "no chat model is installed"
out=$(j "SHELL=/nonexistent OLLAMA_HOST=127.0.0.1:$((PORT+2))")
expect "ollama 401 -> needs API key hint" "$out" "needs an API key"
out=$(j "SHELL=/nonexistent OLLAMA_HOST=127.0.0.1:$((PORT+1))")
case "$out" in *'"model": "bge-m3:latest"'*) bad "bge-m3 (embedding-only) chosen as chat model";; *) ok "embedding model not chosen";; esac

echo "--- agent CLIs"
for n in claude codex llm gemini; do ln -sf "$PWD/fake_agent.sh" "$T/bin/$n"; done
A="SHELL=/nonexistent FAKE_LOG_DIR=$T OLLAMA_HOST=127.0.0.1:$((PORT+3))"
out=$(MZ_ENV="$A" mz providers)
for id in claude-code codex gemini-cli llm; do expect "detect $id" "$out" "$T/bin/"; done
for b in claude-code codex llm gemini-cli; do
  out=$(echo "this are a test" | MZ_ENV="$A" mz run fix --print --backend $b 2>&1); expect "run $b" "$out" "Fixed by"
done
expect "claude argv has --tools ''" "$(cat "$T/claude.log")" $'ARG6=[--tools]\nARG7=[]'
expect "claude gets text on stdin" "$(cat "$T/claude.log")" "this are a test"
expect "codex reads prompt from stdin (-)" "$(grep '^ARG' "$T/codex.log" | tail -1)" "=[-]"
expect "codex shell tool and web search off" "$(cat "$T/codex.log")" "features.shell_tool=false"
expect "llm gets system via -s" "$(cat "$T/llm.log")" "ARG3=[You are a writing assistant"
case "$(grep '^PWD=' "$T/codex.log")" in PWD=/tmp|PWD=/tmp/) bad "child runs in a private dir, not /tmp";; *) ok "child runs in a private dir, not /tmp";; esac
out=$(echo x | MZ_ENV="$A FAKE_MODE=fail" mz run --print --backend codex 2>&1)
expect "non-zero exit + stderr surfaced" "$out" "exit status: 7: fatal: not logged in"
out=$(echo x | MZ_ENV="$A FAKE_MODE=claudefail" mz run --print --backend claude-code 2>&1)
expect "claude is_error result surfaced" "$out" "Invalid API key"
writecfg <<<$'[provider]\ntimeout_secs = 1'

echo x | MZ_ENV="$A FAKE_MODE=slowchild" mz run --print --backend codex >/dev/null 2>&1
sleep 0.5
if pgrep -f "^sleep 31$" >/dev/null; then bad "grandchild of a timed-out agent is left running"; pkill -f "^sleep 31$"; else ok "timed-out agent fully killed"; fi
rm -f "$(cfgfile)"

echo "--- login shell"
H=$T/home
echo 'export DEEPSEEK_API_KEY=sk-profile' > "$H/.profile"
expect "key from ~/.profile via login shell" "$(MZ_ENV="SHELL=$BASH" mz providers 2>/dev/null)" "DEEPSEEK_API_KEY (login shell)"
rm "$H/.profile"; echo 'export DEEPSEEK_API_KEY=sk-bashrc' > "$H/.bashrc"
expect "key from ~/.bashrc (bash) found" "$(MZ_ENV="SHELL=$BASH" mz providers 2>/dev/null)" "DEEPSEEK_API_KEY (login shell)"
rm "$H/.bashrc"; printf 'echo Welcome!\nexport DEEPSEEK_API_KEY=sk-junk\n' > "$H/.profile"
expect "rc chatter on stdout ignored" "$(MZ_ENV="SHELL=$BASH" mz providers 2>/dev/null)" "DEEPSEEK_API_KEY (login shell)"
printf 'sleep 37\n' > "$H/.profile"
s=$(date +%s); MZ_ENV="SHELL=$BASH" mz providers >/dev/null 2>&1; took=$(( $(date +%s) - s ))
[ $took -le 4 ] && ok "hanging rc times out (~3s, took $took)" || bad "hanging rc took $took s"
if pkill -f "^sleep 37$"; then bad "hanging rc's child left running after timeout"; else ok "rc child killed"; fi
rm "$H/.profile"
expect "missing SHELL binary tolerated" "$(MZ_ENV="SHELL=/nope/zsh" mz providers 2>&1; echo "exit=$?")" "exit=0"

echo "--- config editing"
rm -f "$(cfgfile)"
expect "config use creates file" "$(mz config use deepseek 2>&1)" "saved"
[ "$(cat "$(cfgfile)")" = "$(printf '[provider]\nuse = "deepseek"')" ] && ok "new file holds only the setting" || bad "new file holds only the setting" "$(cat "$(cfgfile)")"
expect "unknown id rejected" "$(mz config use nope 2>&1; echo "exit=$?")" "exit=1"
mz config model 'q"uo\te' --backend ollama >/dev/null 2>&1
expect "model with quotes round-trips" "$(MZ_ENV="SHELL=/nonexistent" mz config show)" "ollama = 'q\"uo\\te'"
writecfg <<<'[provider'
expect "config path works with a broken config" "$(mz config path 2>&1; echo "exit=$?")" $'config.toml\nexit=0'
rm -f "$(cfgfile)"

echo "--- errors"
expect "pinned missing backend explains" "$(echo x | MZ_ENV="SHELL=/nonexistent" mz run --print --backend groq 2>&1)" "set GROQ_API_KEY"
expect "unconfigured custom not called unknown" "$(echo x | MZ_ENV="SHELL=/nonexistent" mz run --print --backend custom 2>&1)" "provider.custom"
out=$(MZ_ENV="$MZ_ENV" mz providers --json --check 2>/dev/null)
python3 -c 'import json,sys; json.loads(sys.stdin.read())' <<<"$out" 2>/dev/null && ok "providers --json --check is valid JSON" || bad "providers --json --check mixes text into JSON"

echo "--- grab and paste (fake wl-paste / wl-copy / wtype)"
G=$T/bin; CAT=$(command -v cat); CP=$(command -v cp)
cat > "$G/wl-paste" <<EOF
#!/bin/sh
case "\$*" in *--primary*) $CAT "$T/sel" 2>/dev/null || exit 1;; *) $CAT "$T/clip";; esac
EOF
printf '#!/bin/sh\n%s > "%s/clip"\n' "$CAT" "$T" > "$G/wl-copy"
printf '#!/bin/sh\n%s "%s/clip" "%s/pasted"\n' "$CP" "$T" "$T" > "$G/wtype"
chmod +x "$G/wl-paste" "$G/wl-copy" "$G/wtype"
oai_cfg ok
printf 'old clip' > "$T/clip"
out=$(MZ_ENV="$MZ_ENV MYKEY=sk-test WAYLAND_DISPLAY=wayland-test" mz run fix </dev/null 2>&1; echo "exit=$?")
expect "nothing selected -> exit 1" "$out" $'nothing selected\nexit=1'
printf 'this are a test' > "$T/sel"
out=$(MZ_ENV="$MZ_ENV MYKEY=sk-test WAYLAND_DISPLAY=wayland-test" mz run fix </dev/null 2>&1; echo "exit=$?")
expect "paste via clipboard + Ctrl+V" "$out" "exit=0"
expect "pasted text was the result" "$(cat "$T/pasted" 2>/dev/null)" "Hello, world."
expect "clipboard restored after paste" "$(cat "$T/clip")" "old clip"
expect "selection sent inside <text>" "$(lastreq)" '<text>\nthis are a test\n</text>'
W="$MZ_ENV MYKEY=sk-test WAYLAND_DISPLAY=wayland-test"
expect "history lists the run" "$(MZ_ENV="$W" mz history -n 1)" $'· fix · custom\n  - this are a test\n  + Hello, world.'
MZ_ENV="$W" mz undo </dev/null >/dev/null 2>&1
expect "undo pastes the original back" "$(cat "$T/pasted" 2>/dev/null)" "this are a test"
out=$(echo x | MZ_ENV="$W" mz run summarise 2>/dev/null)
expect "output = show prints instead of pasting" "$out|$(cat "$T/pasted")" "Hello, world.|this are a test"
out=$(echo x | MZ_ENV="$W" mz run key_points 2>/dev/null)
expect "key_points prints (output = show)" "$out|$(lastreq | grep -o 'Extract the key points as a short Markdown')" "Hello, world.|Extract the key points"
echo x | MZ_ENV="$W" mz run translate --target fa --print >/dev/null 2>&1
expect "--target fills {target}" "$(lastreq)" 'code \"fa\"'
# The fake answers --list-types with the clipboard content.
printf 'x-kde-passwordManagerHint' > "$T/clip"
MZ_ENV="$MZ_ENV MYKEY=sk-test WAYLAND_DISPLAY=wayland-test" mz run fix </dev/null >/dev/null 2>&1
expect "password clipboard not restored" "$(cat "$T/clip")" "Hello, world."
rm -f "$G/wl-paste" "$G/wl-copy" "$G/wtype"

echo; echo "passed $pass, failed $fail"
