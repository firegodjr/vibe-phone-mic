#!/usr/bin/env bash
# Tiny uiautomator helper for verification. Usage:
#   e2e/ui.sh dump                 -> prints "resource-id|text|checked|bounds" for app nodes
#   e2e/ui.sh tap <resource-id>    -> taps center of node
#   e2e/ui.sh text <resource-id>   -> prints the node text (or content-desc)
set -euo pipefail
SERIAL="${ANDROID_SERIAL:-emulator-5554}"
A="adb -s $SERIAL"
dumpxml() { $A exec-out uiautomator dump /dev/tty 2>/dev/null | sed 's/UI hierchary dumped.*//'; }
case "$1" in
  dump) dumpxml | python3 -c '
import sys,re,xml.etree.ElementTree as ET
x=sys.stdin.read(); x=x[x.find("<?xml"):]
for n in ET.fromstring(x).iter("node"):
    rid=n.get("resource-id","")
    if rid or n.get("text"):
        print("%s|%s|checked=%s|%s"%(rid,n.get("text") or n.get("content-desc"),n.get("checked"),n.get("bounds")))';;
  tap|text) dumpxml | python3 -c '
import sys,re,xml.etree.ElementTree as ET
x=sys.stdin.read(); x=x[x.find("<?xml"):]
want=sys.argv[1]; mode=sys.argv[2]
for n in ET.fromstring(x).iter("node"):
    if n.get("resource-id")==want:
        if mode=="text":
            t=n.get("text") or ""
            if not t:
                t=" ".join(c.get("text") for c in n.iter("node") if c.get("text"))
            print(t); sys.exit(0)
        a,b,c,d=map(int,re.findall(r"\d+",n.get("bounds")))
        print((a+c)//2,(b+d)//2); sys.exit(0)
sys.exit(3)' "$2" "$1" | { read -r l; if [ "$1" = tap ]; then $A shell input tap $l; else echo "$l"; fi; };;
esac
