"""pm's download-path derivation, reimplemented.

`Step::url_digest` in pm hashes a `dl_urls` key with FNV-1a-64 and uses the
result as a directory name, so a downloaded file lands at
/build/<digest>/<basename>. A recipe has to name that path literally: there is
no shell in a build step, so it cannot be discovered at run time (C1, C9 in
docs/pm-constraints.md).

Nothing in pm promises this layout and no test in pm pins it, so
`tools/check-digest` proves the agreement empirically against a real `pm build`
rather than trusting this file.
"""

OFFSET_BASIS = 0xCBF29CE484222325
PRIME = 0x100000001B3
MASK = 0xFFFFFFFFFFFFFFFF


def url_digest(url: str) -> str:
    """FNV-1a-64 over the URL, formatted exactly as pm formats it."""
    h = OFFSET_BASIS
    for byte in url.encode():
        h = ((h ^ byte) * PRIME) & MASK
    return f"{h:016x}"


def basename(url: str) -> str:
    """The URL's last non-empty path segment, as `Step::download_file_name`.

    pm rejects a URL with no usable name before the build starts; raising here
    turns that into a generation-time error that names the recipe.
    """
    segments = [s for s in url.split("?")[0].split("#")[0].split("/") if s]
    if not segments:
        raise ValueError(f"no file name in URL: {url}")
    return segments[-1]


def download_path(url: str) -> str:
    """Where a dl_urls entry for `url` is readable from, inside the jail."""
    return f"/build/{url_digest(url)}/{basename(url)}"
