#!/bin/sh
# Builds the server from this checkout and runs it with a LaunchAgent: it starts at login, restarts
# if it stops, and is published to your tailnet. Run it on the Mac after pulling changes.
#
#   ./deploy.sh                                   build, install the LaunchAgent, restart
#   PROMPT_SERVER_LLM_MODEL=org/model ./deploy.sh also set the LLM to load when switching to it
#
# An earlier install's LLM model is kept. The password and history in data/ stay as they are.
set -eu
cd "$(dirname "$0")"
here=$(pwd -P)
label=local.comfyui-prompt-server
plist="$HOME/Library/LaunchAgents/$label.plist"
domain="gui/$(id -u)"
log=/tmp/comfyui-prompt-server.log

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Install it from https://rustup.rs (or: brew install rust), then run this again." >&2
  exit 1
fi
if [ ! -f data/auth.json ]; then
  echo "No password is set yet. Run ./start.sh once in Terminal to choose one, then run this again." >&2
  exit 1
fi

echo "Building…"
cargo build --release --locked --quiet

model=${PROMPT_SERVER_LLM_MODEL:-}
if [ -z "$model" ] && [ -f "$plist" ]; then
  model=$(/usr/libexec/PlistBuddy -c 'Print :EnvironmentVariables:PROMPT_SERVER_LLM_MODEL' "$plist" 2>/dev/null || true)
  if [ -z "$model" ]; then
    # Installs before this script passed it as an argument.
    model=$(/usr/libexec/PlistBuddy -c 'Print :ProgramArguments' "$plist" 2>/dev/null |
      awk 'found { print $1; exit } $1 == "--llm-model" { found = 1 }' || true)
  fi
fi

xml() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }
model_entry=""
if [ -n "$model" ]; then
  model_entry="    <key>PROMPT_SERVER_LLM_MODEL</key><string>$(xml "$model")</string>"
fi

mkdir -p "$(dirname "$plist")"
cat > "$plist.new" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>$(xml "$here")/target/release/comfyui-prompt-server</string>
    <string>--tailscale</string>
  </array>
  <key>WorkingDirectory</key><string>$(xml "$here")</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
$model_entry
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
</dict>
</plist>
EOF
plutil -lint -s "$plist.new"
mv "$plist.new" "$plist"

# Stop the running server, wait until launchd lets go of it, then start the new build.
launchctl bootout "$domain/$label" 2>/dev/null || true
tries=0
while launchctl print "$domain/$label" >/dev/null 2>&1 && [ $tries -lt 50 ]; do
  sleep 0.2
  tries=$((tries + 1))
done
start=$(wc -l < "$log" 2>/dev/null || echo 0)
launchctl bootstrap "$domain" "$plist"

tries=0
until curl -fsS -m 2 http://127.0.0.1:8189/health >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ $tries -ge 60 ]; then
    echo "The server didn't answer within a minute. Its log, $log:" >&2
    tail -n +"$((start + 1))" "$log" >&2 || true
    exit 1
  fi
  sleep 1
done
sleep 1
tail -n +"$((start + 1))" "$log" | grep -E '^  (Workflow|Edit|Switch|Open|Problem)' || true
echo "Running from $here${model:+, LLM model $model}. Stop it with: launchctl bootout $domain/$label"
