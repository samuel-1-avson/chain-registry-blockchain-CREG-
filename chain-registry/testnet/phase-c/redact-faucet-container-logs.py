#!/usr/bin/env python3
"""Print faucet container logs after redacting configured environment values."""

import re
import sys


env_path = sys.argv[1]
secrets = []
with open(env_path, encoding="utf-8") as stream:
    for raw_line in stream:
        key, separator, value = raw_line.rstrip("\r\n").partition("=")
        if separator and value and len(value) >= 6:
            secrets.append(value)

for raw_line in sys.stdin:
    line = raw_line.rstrip("\r\n")
    for secret in secrets:
        line = line.replace(secret, "<redacted>")
    line = re.sub(r"(?i)(https?://[^\s/?#]+)[?][^\s#]+", r"\1?<redacted>", line)
    line = re.sub(r"(?i)([?&](?:key|token|apikey|api_key|password|secret)=)[^&\s]+", r"\1<redacted>", line)
    print(line)
