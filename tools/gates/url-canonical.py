#!/usr/bin/env python3
"""Reject any source URL that pm would normalise before hashing it.

pm derives a download's directory from FNV-1a-64 over `Url::as_str()` -- the
*normalised* form rust-url produces, not the string in sources.lock. rust-url
lowercases the scheme and host, drops a default port, resolves dot-segments and
percent-encodes some characters. Wherever it changes the string, this repo's
computed download path is wrong, the tarball lands somewhere else, and the
build fails with ENOENT on a file that was downloaded and verified moments
earlier.

Rather than reimplement rust-url's normalisation, forbid every URL that is not
already in normal form. Under these rules `Url::parse(s).as_str() == s`, so the
string hashed here is provably the one pm hashes.
"""

import sys
from pathlib import Path
from urllib.parse import urlsplit

REPO = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(REPO / "tools" / "lib"))

import yaml  # noqa: E402

ALLOWED_SCHEMES = {"https", "http"}
SAFE = set(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~/+"
)


def problems(url):
    found = []
    parts = urlsplit(url)

    if parts.scheme not in ALLOWED_SCHEMES:
        found.append(f"scheme {parts.scheme!r} is not https (or loopback http)")
    if parts.scheme != parts.scheme.lower():
        found.append("scheme is not lowercase")
    if parts.hostname and parts.hostname != parts.hostname.lower():
        found.append("host is not lowercase")
    if "@" in parts.netloc:
        found.append("contains userinfo")
    if parts.port in (80, 443):
        found.append("states a default port, which rust-url strips")
    if parts.query:
        found.append("has a query string")
    if parts.fragment:
        found.append("has a fragment")
    if "//" in parts.path:
        found.append("path has an empty segment")
    for segment in parts.path.split("/"):
        if segment in (".", ".."):
            found.append("path has a dot-segment, which rust-url resolves")
            break
    bad = sorted({c for c in parts.path if c not in SAFE})
    if bad:
        found.append(
            f"path has character(s) rust-url may percent-encode: {''.join(bad)}"
        )
    # http is tolerated only for the loopback mirror (see tools/serve-sources).
    if parts.scheme == "http" and parts.hostname not in ("127.0.0.1", "localhost"):
        found.append("plain http is only allowed for the loopback source mirror")
    return found


def main():
    lock = REPO / "manifest" / "sources.lock"
    if not lock.exists():
        print("url-canonical: no sources.lock")
        return 0
    doc = yaml.safe_load(lock.read_text()) or {}

    errors = []
    for key, entry in doc.items():
        for problem in problems(entry["url"]):
            errors.append(f"{key}: {problem}\n    {entry['url']}")

    if errors:
        print("url-canonical: FAILED", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1

    print(f"url-canonical: {len(doc)} source URL(s) in rust-url normal form")
    return 0


if __name__ == "__main__":
    sys.exit(main())
