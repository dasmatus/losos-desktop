#!/usr/bin/env python3
"""Exercise only config generation: no repart, swap activation or disk access."""

import configparser
import ctypes
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
PACKAGE = REPO / "recipes/10-core/losos-swap"
SOURCE = PACKAGE / "files/src/losos-swap.c"
CONFIG = "25-swap.conf"

# Link-time wrappers leave the production binary free of test-only environment
# overrides. Short writes and delayed errors exercise atomic publication too.
FIXTURE = r"""
#define _GNU_SOURCE
#include <errno.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysinfo.h>
#include <unistd.h>

static int mode(const char *name) {
    const char *value = getenv("SWAP_TEST_ERROR");
    return value && !strcmp(value, name);
}
int __wrap_sysinfo(struct sysinfo *info) {
    if (mode("sysinfo")) { errno = EIO; return -1; }
    memset(info, 0, sizeof(*info));
    info->totalram = strtoul(getenv("SWAP_TEST_TOTAL"), NULL, 10);
    info->freeram = strtoul(getenv("SWAP_TEST_FREE"), NULL, 10);
    info->mem_unit = (unsigned int)strtoul(getenv("SWAP_TEST_UNIT"), NULL, 10);
    return 0;
}
ssize_t __real_write(int, const void *, size_t);
ssize_t __wrap_write(int fd, const void *data, size_t length) {
    static unsigned int calls;
    ++calls;
    if (mode("write") && calls > 1) { errno = ENOSPC; return -1; }
    if (mode("zero")) return 0;
    if (mode("interrupt") && calls == 1) { errno = EINTR; return -1; }
    return __real_write(fd, data, length > 7 ? 7 : length);
}
int __real_fsync(int);
int __wrap_fsync(int fd) {
    if (mode("fsync")) { errno = EIO; return -1; }
    return __real_fsync(fd);
}
int __real_fchmod(int, mode_t);
int __wrap_fchmod(int fd, mode_t permissions) {
    if (mode("fchmod")) { errno = EPERM; return -1; }
    return __real_fchmod(fd, permissions);
}
int __real_close(int);
int __wrap_close(int fd) {
    struct stat st;
    int regular = fstat(fd, &st) == 0 && S_ISREG(st.st_mode);
    int result = __real_close(fd);
    if (regular && mode("close")) { errno = EIO; return -1; }
    if (!regular && mode("dirclose")) { errno = EIO; return -1; }
    return result;
}
int __real_renameat(int, const char *, int, const char *);
int __wrap_renameat(int olddir, const char *old, int newdir, const char *new) {
    if (mode("rename")) { errno = EIO; return -1; }
    return __real_renameat(olddir, old, newdir, new);
}
"""


class SwapTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Keep scratch data in the checkout, never on real device paths.
        cls.scratch = tempfile.TemporaryDirectory(prefix=".test-swap-", dir=PACKAGE)
        cls.addClassCleanup(cls.scratch.cleanup)
        cls.root = Path(cls.scratch.name)
        fixture = cls.root / "sysinfo.c"
        fixture.write_text(FIXTURE)
        cls.binary = cls.root / "losos-swap"
        command = shlex.split(os.environ.get("CC", "cc"))
        command += ["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic",
                    str(SOURCE), str(fixture), "-o", str(cls.binary)]
        command += [f"-Wl,--wrap={name}" for name in
                    ("sysinfo", "write", "fsync", "fchmod", "close", "renameat")]
        subprocess.run(command, check=True)

    def setUp(self):
        self.directory = self.root / self._testMethodName
        self.directory.mkdir()
        self.output = self.directory / "nested" / "repart.d"

    def run_helper(self, *args, total=12345, free=1, unit=4096, error="", ok=True):
        environment = dict(os.environ, SWAP_TEST_TOTAL=str(total),
                           SWAP_TEST_FREE=str(free), SWAP_TEST_UNIT=str(unit),
                           SWAP_TEST_ERROR=error)
        result = subprocess.run([str(self.binary), *map(str, args)],
                                env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, ok, result.stderr)
        if not ok:
            self.assertTrue(result.stderr)
        return result

    def assert_config(self, size):
        self.assertEqual((self.output / CONFIG).read_text(),
                         "[Partition]\nType=swap\nLabel=losos-swap\nFormat=swap\n"
                         f"SizeMinBytes={size}\nSizeMaxBytes={size}\n")
        self.assertEqual((self.output / CONFIG).stat().st_mode & 0o777, 0o644)
        self.assertEqual(list(self.output.iterdir()), [self.output / CONFIG])

    def test_total_not_free_and_repeat(self):
        self.run_helper(self.output, free=0)
        self.assert_config(12345 * 4096)
        # Replacing the inode proves the published file was not truncated in place.
        with (self.output / CONFIG).open() as previous:
            old_inode = os.fstat(previous.fileno()).st_ino
            self.run_helper(self.output, total=99, free=98)
            self.assertNotEqual((self.output / CONFIG).stat().st_ino, old_inode)
            self.assertIn(str(12345 * 4096), previous.read())
        self.assert_config(99 * 4096)

    def test_rounding(self):
        for total, unit in ((1, 1), (4096, 1), (4097, 1), (12345, 3),
                            (3 * 1024**3 + 17, 1)):
            with self.subTest(total=total, unit=unit):
                self.run_helper(self.output, total=total, unit=unit)
                self.assert_config((total * unit + 4095) // 4096 * 4096)

    def test_arguments(self):
        for arguments in ((), ("",), (self.output, "extra")):
            with self.subTest(arguments=arguments):
                self.run_helper(*arguments, ok=False)
        self.assertFalse(self.output.exists())

    def test_invalid_sysinfo(self):
        cases = [dict(total=0), dict(unit=0), dict(error="sysinfo")]
        if ctypes.sizeof(ctypes.c_ulong) == 8:
            cases += [dict(total=2**64 - 1, unit=2),
                      dict(total=2**64 - 1, unit=1)]
        for case in cases:
            with self.subTest(case=case):
                self.run_helper(self.output, ok=False, **case)
                self.assertFalse(self.output.exists())

    @unittest.skipUnless(ctypes.sizeof(ctypes.c_ulong) == 8, "64-bit sysinfo")
    def test_largest_aligned_size(self):
        self.run_helper(self.output, total=2**64 - 4096, unit=1)
        self.assert_config(2**64 - 4096)

    def test_interrupted_short_writes(self):
        self.run_helper(self.output, error="interrupt")
        self.assert_config(12345 * 4096)

    def test_write_errors_preserve_old_config(self):
        self.run_helper(self.output)
        for error in ("write", "zero", "fchmod", "fsync", "close", "rename"):
            with self.subTest(error=error):
                self.run_helper(self.output, total=1, error=error, ok=False)
                self.assert_config(12345 * 4096)

    def test_write_errors_leave_no_partial_config(self):
        for error in ("write", "zero", "fchmod", "fsync", "close", "rename"):
            with self.subTest(error=error):
                self.run_helper(self.output, error=error, ok=False)
                self.assertEqual(list(self.output.iterdir()), [])

    def test_directory_close_error_is_not_silent(self):
        self.run_helper(self.output, error="dirclose", ok=False)
        self.assertTrue((self.output / CONFIG).is_file())

    def test_symlink_directories(self):
        outside = self.directory / "outside"
        outside.mkdir()
        alias = self.directory / "alias"
        alias.symlink_to(outside, target_is_directory=True)
        for path in (alias, alias / "nested"):
            self.run_helper(path, ok=False)
        self.assertEqual(list(outside.iterdir()), [])

    def test_unsafe_destinations(self):
        self.output.mkdir(mode=0o755, parents=True)
        target = self.directory / "untouched"
        target.write_text("sentinel")
        config = self.output / CONFIG
        for destination in (target, self.directory / "missing"):
            config.symlink_to(destination)
            self.run_helper(self.output, ok=False)
            self.assertTrue(config.is_symlink())
            config.unlink()
        config.mkdir()
        self.run_helper(self.output, ok=False)
        config.rmdir()
        os.mkfifo(config)
        self.run_helper(self.output, ok=False)
        config.unlink()
        self.assertEqual(target.read_text(), "sentinel")
        self.assertFalse((self.directory / "missing").exists())
        self.assertEqual(list(self.output.iterdir()), [])

    def test_hardlink_is_replaced_not_modified(self):
        self.output.mkdir(mode=0o755, parents=True)
        target = self.directory / "untouched"
        target.write_text("sentinel")
        os.link(target, self.output / CONFIG)
        self.run_helper(self.output)
        self.assertEqual(target.read_text(), "sentinel")
        self.assert_config(12345 * 4096)

    def test_invalid_directory_paths(self):
        file = self.directory / "file"
        file.write_text("sentinel")
        self.run_helper(file, ok=False)
        self.run_helper(file / "nested", ok=False)
        self.run_helper(self.directory / ".." / "escape", ok=False)
        self.output.mkdir(mode=0o755, parents=True)
        self.output.chmod(0o777)
        self.run_helper(self.output, ok=False)
        self.assertEqual(list(self.output.iterdir()), [])
        self.assertEqual(file.read_text(), "sentinel")


class SwapWiringTests(unittest.TestCase):
    @staticmethod
    def active_lines(path):
        return [line.strip() for line in (REPO / path).read_text().splitlines()
                if line.strip() and not line.lstrip().startswith("#")]

    def test_build_and_gate_membership(self):
        # Keep this stdlib-only: templates are not generally parseable YAML.
        layers = "\n".join(self.active_lines("manifest/layers.yaml"))
        core = next(block for block in re.split(r"(?m)^- name: ", layers)
                    if block.startswith("losos-05-core\n"))
        self.assertIn("dir: 10-core", core.splitlines())
        self.assertIn("- losos-swap", core.split("members:\n", 1)[1].splitlines())
        justfile = (REPO / "Justfile").read_text()
        lint = re.search(r"(?ms)^lint:\n(.*?)(?=^\S|\Z)", justfile)
        self.assertIsNotNone(lint)
        self.assertIn('python3 "{{repo}}/tools/gates/test-swap.py"',
                      [line.strip() for line in lint[1].splitlines()])

    def test_recipe_source_layout(self):
        # configure already copies the member's files/ to files/<package>/;
        # a second authored package directory would leave setup without meson.build.
        self.assertTrue((PACKAGE / "files/meson.build").is_file())
        self.assertTrue(SOURCE.is_file())
        self.assertFalse((PACKAGE / "files/losos-swap").exists())
        meson = (PACKAGE / "files/meson.build").read_text()
        self.assertIn("'src/losos-swap.c'", meson)
        recipe = (PACKAGE / "build.yaml.in").read_text()
        self.assertIn("setup /build/b/losos-swap @RECIPE@/files/losos-swap ", recipe)

    def test_initrd_programs(self):
        files = self.active_lines("recipes/90-image/losos-image/files/initrd-manifest.txt")
        for path in ("usr/lib/repart.d", "usr/lib/repart.sysinstall.d",
                     "usr/lib/systemd/systemd-sysinstall",
                     "usr/lib/losos/losos-swap", "usr/sbin/mkswap",
                     "usr/sbin/swapon", "usr/sbin/swapoff"):
            with self.subTest(path=path):
                self.assertIn(path, files)

    def test_service_ordering_and_installer_isolation(self):
        for service, dropin, output, condition in (
            ("systemd-repart", "20-swap.conf", "/sysroot/run/repart.d", "!losos.install"),
            ("systemd-sysinstall", "10-losos.conf", "/run/repart.sysinstall.d",
             "losos.install"),
        ):
            with self.subTest(service=service):
                config = configparser.ConfigParser(interpolation=None, strict=False)
                config.optionxform = str
                config.read(REPO / "overlay/usr/lib/systemd/system" /
                            f"{service}.service.d" / dropin)
                self.assertEqual(config["Unit"]["ConditionKernelCommandLine"], condition)
                self.assertEqual(config["Service"]["ExecStartPre"],
                                 f"/usr/lib/losos/losos-swap {output}")
                if service == "systemd-repart":
                    self.assertNotIn("ExecStart", config["Service"])
                    lines = self.active_lines(
                        f"overlay/usr/lib/systemd/system/{service}.service.d/{dropin}")
                    self.assertFalse(any(line.startswith("ExecStart=") for line in lines))

    def test_kernel_swap_and_zswap(self):
        lines = self.active_lines("recipes/10-systemd/linux/files/losos.config")
        for symbol in ("SWAP", "ZSWAP", "ZSWAP_DEFAULT_ON",
                       "ZSWAP_COMPRESSOR_DEFAULT_842", "CRYPTO_842", "ZSMALLOC"):
            with self.subTest(symbol=symbol):
                settings = [line for line in lines if line.startswith(f"CONFIG_{symbol}=")]
                self.assertEqual(settings, [f"CONFIG_{symbol}=y"])
        recipe = self.active_lines("recipes/10-systemd/linux/build.yaml.in")
        for symbol in ("SWAP", "ZSWAP", "ZSWAP_DEFAULT_ON",
                       "ZSWAP_COMPRESSOR_DEFAULT_842", "CRYPTO_842", "ZSMALLOC"):
            with self.subTest(symbol=symbol):
                self.assertIn(f"- grep -qx CONFIG_{symbol}=y /build/b/linux/.config", recipe)

    def test_swap_utilities_enabled_and_patched(self):
        recipe = "recipes/00-base/util-linux"
        lines = self.active_lines(f"{recipe}/build.yaml.in")
        configure = next(line for line in lines
                         if "/build/src/util-linux/configure " in line)
        arguments = configure.split()
        for argument in ("--enable-swapon", "--enable-libsmartcols",
                         "--sbindir=/usr/sbin"):
            self.assertIn(argument, arguments)
        # 2.40 has no --enable-mkswap. The shipped configure needs the patch,
        # applied before configuration, not merely a patch file in the tree.
        self.assertNotIn("--enable-mkswap", arguments)
        patch_command = (
            "- /bin/sh @RECIPE@/apply-patches.sh "
            "@RECIPE@/files/util-linux/patches /build/src/util-linux"
        )
        self.assertLess(lines.index(patch_command), lines.index(configure))
        patch = (REPO / recipe / "files/patches" /
                 "0001-build-mkswap-with-swap-tools.patch").read_text()
        self.assertIn("+++ b/configure\n", patch)
        self.assertIn("-    build_mkswap=no\n+    build_mkswap=yes\n", patch)
        for program in ("mkswap", "swapon", "swapoff"):
            self.assertIn(f"- test -x /dest/usr/sbin/{program}", lines)


if __name__ == "__main__":
    unittest.main()
