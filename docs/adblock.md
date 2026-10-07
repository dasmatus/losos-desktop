# System-wide ad blocking

Every app on the OS gets ads and trackers blocked from the same
EasyList-style filter lists, by `losos-adblock` (`src/losos-adblock`,
`nixos/modules/adblock.nix`). It has two halves.

**DNS, for every app.** resolved sends every name to losos-adblock's
forwarder on `127.0.0.153:53`, its only global DNS server, and the
networks' own servers no longer route anything by themselves. The
forwarder answers `0.0.0.0` (A) or `::` (AAAA) for a blocked domain and
its subdomains, and no records for any other type, so a program fails
at once rather than retrying, and passes every other query on, byte for
byte, to the servers the network gave (it reads them from
`/run/systemd/resolve/resolv.conf`). That covers Flatpaks, the Android
layer and anything else that resolves names, but only whole domains. A
network's own search domains still go straight to its servers, so local
names keep working. resolved's DNS over TLS is off, since a forwarder on
loopback speaks plain DNS; the forwarder's own queries are plain DNS, which
is what "opportunistic" fell back to on most networks anyway.

**Content blockers, in the browser.** The rules that need a URL or a page
(a path such as `/banner/ad_`, an element to hide such as
`example.org##.sponsored`) are compiled with Brave's `adblock` crate into
WebKit content blocker rule sets, which [Danube](danube.md) loads into
every page. Rules the DNS half already applies are left out, so WebKit
does not compile them twice.

## Lists

`losos.adblock.lists` names them, by https URL or absolute path; the
defaults are EasyList, EasyPrivacy, AdGuard's DNS filter and Peter Lowe's
list. Adblock Plus syntax, hosts files and plain domain lists all work. A
user adds more, one per line, in `/var/lib/losos-adblock/lists`.

`losos-adblock-update.timer` fetches them two minutes after boot and then
daily, with `If-None-Match` so an unchanged list is not downloaded again,
keeping the last good copy of a list that fails, and compiles both
halves. `losos-adblock update` does the same by hand, and
`losos-adblock check NAME...` says whether a name is blocked and by which
rule.

## Exceptions

`losos.adblock.allow` and `/var/lib/losos-adblock/allow` list domains
never blocked, with their subdomains, whatever the lists say; in the
browser, nothing is blocked on their pages either. The lists' own `@@`
exceptions apply in both halves too.

## Files

| Path | What |
| --- | --- |
| `/etc/losos-adblock/lists`, `allow` | From the module |
| `/var/lib/losos-adblock/lists`, `allow` | The user's additions |
| `/var/lib/losos-adblock/lists/` | The fetched copies |
| `/var/lib/losos-adblock/dns.list` | The compiled domain list the forwarder reads |
| `/var/lib/losos-adblock/webkit/` | WebKit rule sets and their `index` |

With the four default lists the domain list has about 195,000 names, and
the WebKit sets come to about 5 MB of JSON, which WebKit compiles once and
keeps in `~/.cache/danube/content-filters`.
