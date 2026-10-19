#!/bin/sh
# Serves the app to Expo Go on your iPhone. When Tailscale is up, Expo Go reaches this Mac by its
# tailnet name, so the app opens anywhere the iPhone has Tailscale, not only on the same Wi-Fi.
set -e
cd "$(dirname "$0")"
[ -d node_modules ] || npm install
TS=tailscale
command -v tailscale >/dev/null 2>&1 || TS=/Applications/Tailscale.app/Contents/MacOS/Tailscale
NAME=$("$TS" status --json 2>/dev/null | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{console.log(JSON.parse(s).Self.DNSName.replace(/\.$/,""))}catch{}})' || true)
if [ -n "$NAME" ]; then
  export REACT_NATIVE_PACKAGER_HOSTNAME="$NAME"
  echo "Expo Go will load the app from $NAME over Tailscale."
fi
exec npx expo start "$@"
