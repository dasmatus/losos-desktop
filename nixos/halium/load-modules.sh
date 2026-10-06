# Load the kernel modules an Android partition lists, as Android's own init
# would: every module named in <dir>/modules.load, with the parameters
# modules.options gives it, skipping what modules.blocklist names.
#
# The device's kernel is the one its maker built, and its drivers are
# modules beside it: vendor_boot's ramdisk holds the ones needed to reach
# storage, system_dlkm the generic kernel's own, and vendor_dlkm (or vendor on
# older devices) the rest. None of them carries a modules.dep.bin, so kmod's
# modprobe cannot resolve their dependencies; insmod in passes does, retrying
# a module whose symbols another has not exported yet until a pass loads
# nothing new. A module that never loads is reported and skipped, never fatal:
# a missing camera driver must not stop the phone from booting.
#
#   load-modules <dir>...

load() {
  local dir=$1 line name path args
  [ -f "$dir/modules.load" ] || return 0

  declare -A options=() blocked=()
  if [ -f "$dir/modules.options" ]; then
    while read -r line; do
      # options <name> <parameters...>
      read -r _ name args <<<"$line"
      [ -n "$name" ] && options[${name//-/_}]=$args
    done < <(grep '^options ' "$dir/modules.options")
  fi
  if [ -f "$dir/modules.blocklist" ]; then
    while read -r _ name; do
      [ -n "$name" ] && blocked[${name//-/_}]=1
    done < <(grep '^blocklist ' "$dir/modules.blocklist")
  fi

  local pending=() next=() progress
  while read -r line; do
    [ -n "$line" ] && pending+=("$line")
  done <"$dir/modules.load"

  while [ ${#pending[@]} -gt 0 ]; do
    next=()
    progress=
    for line in "${pending[@]}"; do
      name=$(basename "$line" .ko)
      name=${name//-/_}
      # Already loaded, or built into this kernel.
      if [ -n "${blocked[$name]:-}" ] || [ -e "/sys/module/$name" ]; then
        continue
      fi
      # An entry is a path inside the partition (kernel/drivers/...), a bare
      # file name beside modules.load, or, in some vendor trees, absolute.
      case $line in
        /*) path=$line ;;
        *) path=$dir/$line ;;
      esac
      [ -f "$path" ] || path=$dir/$(basename "$line")
      # shellcheck disable=SC2086 # the options are several words on purpose
      if insmod "$path" ${options[$name]:-} 2>/dev/null; then
        progress=1
      else
        next+=("$line")
      fi
    done
    [ -n "$progress" ] || break
    pending=("${next[@]}")
  done

  for line in "${next[@]}"; do
    name=$(basename "$line" .ko)
    [ -e "/sys/module/${name//-/_}" ] || echo "$dir: $line did not load" >&2
  done
}

for dir in "$@"; do
  load "$dir"
done
