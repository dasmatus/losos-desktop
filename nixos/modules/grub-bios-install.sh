# losos-grub-bios-install DISK [DIR]: makes DISK start GRUB on a legacy BIOS
# PC.
#
# DIR holds GRUB's boot.img and core.img, the ones bios.nix builds unless
# another is named; bios.nix sets $boot_code to them. boot.img goes in the
# first 440 bytes of the protective MBR, which UEFI firmware ignores and the
# partition table after it keeps; core.img goes in the disk's BIOS boot
# partition, which nothing else reads. DISK may be a block device (the
# installer) or a disk image file (image.nix).
#
# This is what grub-bios-setup does when it embeds core.img in a BIOS boot
# partition, written out because grub-bios-setup insists on probing the
# filesystem GRUB's modules would be read from, and this GRUB carries its
# modules and its menu inside core.img and reads no such filesystem.

disk=$1
dir=${2:-$boot_code}

# Where the BIOS boot partition starts and how long it is, in 512-byte
# sectors, as sfdisk reports it for the GPT.
read -r start size < <(
  sfdisk --json "$disk" | jq -r '
    .partitiontable as $t
    | ($t.sectorsize // 512) as $s
    | [$t.partitions[] | select(.type == "21686148-6449-6E6F-744E-656564454649")][0]
    | if . == null then error("no BIOS boot partition") else . end
    | "\(.start * $s / 512) \(.size * $s / 512)"'
)

if [ -z "${start:-}" ]; then
  echo "$disk has no BIOS boot partition" >&2
  exit 1
fi

core_bytes=$(stat -c %s "$dir/core.img")
core_sectors=$(((core_bytes + 511) / 512))
if [ "$core_sectors" -gt "$size" ]; then
  echo "core.img is $core_sectors sectors and the BIOS boot partition only $size" >&2
  exit 1
fi

# Little-endian integers, as the x86 boot code reads them.
le() {
  local value=$1 bytes=$2 i out=
  for ((i = 0; i < bytes; i++)); do
    out+=$(printf '\\x%02x' $(((value >> (8 * i)) & 255)))
  done
  printf '%b' "$out"
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# core.img's first sector (diskboot.img) loads the rest from the blocklist
# at its end: the sector after it, how many follow, and the segment they go
# to (GRUB_BOOT_I386_PC_KERNEL_SEG + 0x20), then a zeroed terminator.
cp "$dir/core.img" "$work/core.img"
chmod u+w "$work/core.img"
{
  le $((start + 1)) 8
  le $((core_sectors - 1)) 2
  le $((0x820)) 2
} | dd of="$work/core.img" bs=1 seek=500 conv=notrunc status=none
truncate -s $((core_sectors * 512)) "$work/core.img"

# boot.img: where core.img starts (GRUB_BOOT_MACHINE_KERNEL_SECTOR), the
# drive the BIOS says it booted from (0xff), and the check that drive is a
# hard disk replaced with two nops, for BIOSes that pass a wrong one.
cp "$dir/boot.img" "$work/boot.img"
chmod u+w "$work/boot.img"
le "$start" 8 | dd of="$work/boot.img" bs=1 seek=$((0x5c)) conv=notrunc status=none
printf '\xff' | dd of="$work/boot.img" bs=1 seek=$((0x64)) conv=notrunc status=none
printf '\x90\x90' | dd of="$work/boot.img" bs=1 seek=$((0x66)) conv=notrunc status=none

dd if="$work/core.img" of="$disk" bs=512 seek="$start" conv=notrunc,fsync status=none
dd if="$work/boot.img" of="$disk" bs=440 count=1 conv=notrunc,fsync status=none
