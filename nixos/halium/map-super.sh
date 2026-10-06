# Map the logical partitions of this slot out of `super` with device-mapper,
# as Android's first-stage init does, so /dev/mapper/vendor and the rest
# exist for the mounts in android.nix.
#
# Every device that launched with Android 10 or later keeps vendor, odm,
# vendor_dlkm and the rest as extents of one physical `super` partition,
# described by liblp metadata at its start. lpdump prints each partition's
# extents in 512-byte sectors, which is exactly a device-mapper linear
# table: `<start> .. <end> linear super <offset>` becomes
# `<start> <length> linear /dev/...super <offset>`. Only extents on super
# itself are mapped; a retrofit device, whose logical partitions span old
# physical ones, launched before Android 13 and so is not a GSI device.
#
#   map-super <super block device>

super=$1

# The bootloader names the slot it booted in bootconfig (Android 12 and
# later) or on the kernel command line. No suffix is a device without A/B,
# whose one metadata slot is 0.
suffix=
for source in /proc/bootconfig /proc/cmdline; do
  [ -r "$source" ] || continue
  found=$(grep -o 'androidboot\.slot_suffix *= *"\?_[ab]' "$source" | grep -o '_[ab]$' || true)
  if [ -n "$found" ]; then
    suffix=$found
    break
  fi
done

lpdump --slot="${suffix:-0}" "$super" | awk -v suffix="$suffix" -v super="$super" '
  # Write each finished partition as "name<TAB>table line;table line;...".
  function flush() {
    if (name != "" && table != "") print name "\t" table
    name = ""; table = ""
  }
  /^  Name: / {
    flush()
    n = $2
    # Only this slot'"'"'s partitions, named without the suffix, so the
    # mounts say /dev/mapper/vendor on either slot.
    if (suffix == "" || substr(n, length(n) - 1) == suffix) {
      name = suffix == "" ? n : substr(n, 1, length(n) - 2)
    }
    next
  }
  # "    0 .. 16383 linear super 2048"
  name != "" && $2 == ".." && $4 == "linear" {
    if ($5 != "super") { name = ""; table = ""; next }
    line = $1 " " ($3 - $1 + 1) " linear " super " " $6
    table = table == "" ? line : table ";" line
  }
  /^-+$/ { flush() }
  END { flush() }
' | while IFS=$'\t' read -r name table; do
  [ -e "/dev/mapper/$name" ] && continue
  tr ';' '\n' <<<"$table" | dmsetup create --readonly "$name"
  echo "mapped $name$suffix"
done
