#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysinfo.h>
#include <unistd.h>

#define CONFIG "25-swap.conf"
#define GRAIN UINT64_C(4096)

/* Walk by directory descriptors: O_NOFOLLOW on only the last component would
 * still let an ancestor symlink redirect a privileged write. */
static int output_directory(const char *path)
{
    char *copy = strdup(path), *save = NULL;
    int fd = -1;
    if (!copy)
        return -1;
    fd = open(path[0] == '/' ? "/" : ".", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (fd < 0)
        goto fail;
    for (char *part = strtok_r(copy, "/", &save); part;
         part = strtok_r(NULL, "/", &save)) {
        if (!strcmp(part, "..")) {
            errno = EINVAL;
            goto fail;
        }
        if (!strcmp(part, "."))
            continue;
        if (mkdirat(fd, part, 0755) < 0 && errno != EEXIST)
            goto fail;
        int next = openat(fd, part, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        if (next < 0)
            goto fail;
        close(fd);
        fd = next;
    }
    /* A shared writable output directory could let another user replace the
     * staging name between writing it and renaming it. */
    struct stat st;
    if (fstat(fd, &st) < 0)
        goto fail;
    if (st.st_uid != geteuid() || (st.st_mode & 0022)) {
        errno = EPERM;
        goto fail;
    }
    free(copy);
    return fd;

fail:
    {
        int error = errno;
        if (fd >= 0)
            close(fd);
        free(copy);
        errno = error;
        return -1;
    }
}

static int write_config(int dir, uint64_t bytes)
{
    char text[256], staging[64] = "";
    int fd = -1, result = -1;
    struct stat st;
    if (fstatat(dir, CONFIG, &st, AT_SYMLINK_NOFOLLOW) == 0) {
        if (!S_ISREG(st.st_mode)) {
            errno = EINVAL;
            return -1;
        }
    } else if (errno != ENOENT) {
        return -1;
    }
    int length = snprintf(text, sizeof(text),
                          "[Partition]\nType=swap\nLabel=losos-swap\nFormat=swap\n"
                          "SizeMinBytes=%" PRIu64 "\nSizeMaxBytes=%" PRIu64 "\n",
                          bytes, bytes);
    if (length < 0 || (size_t)length >= sizeof(text)) {
        errno = EOVERFLOW;
        return -1;
    }
    /* Exclusive creation never truncates a pre-existing file or follows a link.
     * The hidden name does not end in .conf, so repart cannot read partial data. */
    for (unsigned int attempt = 0; attempt < 128; ++attempt) {
        snprintf(staging, sizeof(staging), ".25-swap.conf.%ld.%u",
                 (long)getpid(), attempt);
        fd = openat(dir, staging, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                    0600);
        if (fd >= 0)
            break;
        if (errno != EEXIST)
            return -1;
    }
    if (fd < 0)
        return -1;
    size_t offset = 0;
    while (offset < (size_t)length) {
        ssize_t written = write(fd, text + offset, (size_t)length - offset);
        if (written < 0 && errno == EINTR)
            continue;
        if (written <= 0) {
            if (written == 0)
                errno = EIO;
            goto done;
        }
        offset += (size_t)written;
    }
    if (fchmod(fd, 0644) < 0 || fsync(fd) < 0)
        goto done;
    /* Close before publishing, so even a delayed write error leaves the old
     * complete config untouched. Rename replaces hard links rather than writing
     * through them, and never follows a concurrently substituted symlink. */
    result = close(fd);
    fd = -1;
    if (result < 0)
        goto done;
    result = renameat(dir, staging, dir, CONFIG);
done:
    {
        int error = errno;
        if (fd >= 0)
            close(fd);
        if (result < 0)
            unlinkat(dir, staging, 0);
        errno = error;
        return result;
    }
}

int main(int argc, char **argv)
{
    struct sysinfo info;
    if (argc != 2 || !argv[1][0]) {
        fprintf(stderr, "usage: losos-swap OUTPUT-DIRECTORY\n");
        return EXIT_FAILURE;
    }
    if (sysinfo(&info) < 0) {
        perror("losos-swap: sysinfo");
        return EXIT_FAILURE;
    }
    /* totalram is usable RAM, not the momentary free RAM. Reject missing units
     * rather than silently guessing a size, and check both multiply and ceil. */
    if (!info.totalram || !info.mem_unit ||
        (uint64_t)info.totalram > UINT64_MAX / info.mem_unit) {
        fprintf(stderr, "losos-swap: invalid or overflowing RAM size\n");
        return EXIT_FAILURE;
    }
    uint64_t bytes = (uint64_t)info.totalram * info.mem_unit;
    if (bytes > UINT64_MAX - (GRAIN - 1)) {
        fprintf(stderr, "losos-swap: rounded RAM size overflows\n");
        return EXIT_FAILURE;
    }
    bytes = (bytes + GRAIN - 1) / GRAIN * GRAIN;
    int dir = output_directory(argv[1]);
    if (dir < 0) {
        perror("losos-swap: output directory");
        return EXIT_FAILURE;
    }
    int result = write_config(dir, bytes);
    if (result < 0)
        perror("losos-swap: write " CONFIG);
    int close_result = close(dir);
    if (close_result < 0 && result >= 0) {
        perror("losos-swap: close output directory");
        return EXIT_FAILURE;
    }
    return result < 0 ? EXIT_FAILURE : EXIT_SUCCESS;
}
