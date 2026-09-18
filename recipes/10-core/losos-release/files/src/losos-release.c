/* Print the fields of /usr/lib/os-release.
 *
 * systemd reads os-release in a dozen places -- systemd-firstboot seeds a
 * machine from it, systemd-boot puts PRETTY_NAME in the menu entry, the UKI
 * carries a copy in its .osrel section so the stub can identify itself before
 * any filesystem is mounted. This reads the same file back, which makes it a
 * useful smoke test on a running system and a real compile at build time.
 *
 * Deliberately freestanding of everything but libc: this is the bottom of the
 * tree, and anything it linked against would have to be built first.
 */

/* meson's c_std=c11 means -std=c11, which is strict ISO and hides every POSIX
 * declaration -- readlink(), access() and PATH_MAX among them. Ask for POSIX
 * 2008 explicitly rather than relaxing the standard to gnu11: the code is
 * meant to be ISO C plus a named POSIX surface, not whatever GNU adds.
 * The macro must precede every include. */
#define _POSIX_C_SOURCE 200809L

#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "config.h"

#define LINE_MAX_LEN 4096

/* Resolve os-release relative to this executable, falling back to the
 * configured absolute path.
 *
 * The compiled-in default is /usr/lib/os-release, and that is right on a
 * running LosOS. It is wrong everywhere else the binary might be examined:
 * pm's run jail extracts a package at /pkg and ALSO mirrors the host's /usr
 * read-only, so the absolute path silently resolves to the build host's
 * os-release and this program cheerfully reports the wrong distribution. The
 * same trap catches anything inspecting the staged tree before installation.
 *
 * /proc/self/exe costs nothing and makes the package relocatable, which is
 * what it should have been anyway.
 */
static const char *
find_os_release(char *buf, size_t buflen)
{
    ssize_t n = readlink("/proc/self/exe", buf, buflen - 1);
    if (n <= 0)
        return LOSOS_OSRELEASE;
    buf[n] = '\0';

    /* .../usr/bin/losos-release -> .../usr/lib/os-release */
    char *slash = strrchr(buf, '/');
    if (slash == NULL)
        return LOSOS_OSRELEASE;
    *slash = '\0';
    slash = strrchr(buf, '/');
    if (slash == NULL || strcmp(slash, "/bin") != 0)
        return LOSOS_OSRELEASE;
    *slash = '\0';

    if (strlen(buf) + sizeof "/lib/os-release" > buflen)
        return LOSOS_OSRELEASE;
    strcat(buf, "/lib/os-release");

    if (access(buf, R_OK) != 0)
        return LOSOS_OSRELEASE;
    return buf;
}

static void
print_field(const char *line, const char *key)
{
    size_t keylen = strlen(key);

    if (strncmp(line, key, keylen) != 0 || line[keylen] != '=')
        return;

    const char *value = line + keylen + 1;

    /* os-release values may be quoted; the quotes are syntax, not content. */
    size_t len = strlen(value);
    if (len >= 2 && value[0] == '"' && value[len - 1] == '"') {
        printf("%s: %.*s\n", key, (int)(len - 2), value + 1);
        return;
    }

    printf("%s: %s\n", key, value);
}

int
main(int argc, char **argv)
{
    char resolved[PATH_MAX];
    const char *path = (argc > 1) ? argv[1]
                                  : find_os_release(resolved, sizeof resolved);

    FILE *f = fopen(path, "re");
    if (f == NULL) {
        fprintf(stderr, "losos-release: cannot open %s\n", path);
        return 1;
    }

    char line[LINE_MAX_LEN];
    while (fgets(line, sizeof line, f) != NULL) {
        line[strcspn(line, "\n")] = '\0';
        print_field(line, "PRETTY_NAME");
        print_field(line, "ID");
        print_field(line, "VERSION_ID");
    }

    fclose(f);
    printf("built-by: losos-desktop %s\n", LOSOS_VERSION);
    return 0;
}
