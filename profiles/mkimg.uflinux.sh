# shellcheck shell=sh disable=SC2034,SC3043
# These variables are consumed by the upstream mkimage plugin loader.
profile_uflinux() {
	profile_standard
	# Retain upstream boot support, but keep competing network/clock stacks off media.
	local selected="" package
	for package in $apks; do
		case "$package" in chrony|openntpd|dhcpcd|wpa_supplicant|network-extras|tiny-cloud-alpine) ;; *) selected="$selected $package" ;; esac
	done
	apks="$selected"
	title="UFLinux"
	desc="Minimal Alpine live desktop with labwc and Ironbar"
	image_name="uflinux"
	profile_abbrev="uflinux"
	hostname="uflinux"
	output_filename="uflinux-${RELEASE}-${ARCH}.iso"
	apkovl="/project/profiles/genapkovl-uflinux.sh"
	# Keep profile_standard's architecture-specific console and boot configuration.
	kernel_addons=""
	initfs_features="$initfs_features kms xfs"
	apks="$apks uflinux uflinux-live uflinux-installer linux-lts linux-firmware-none
		grub-efi xfsprogs e2fsprogs dosfstools util-linux vulkan-loader mesa-vulkan-ati"
	case "$ARCH" in
		x86|x86_64) apks="$apks mesa-vulkan-intel mesa-vulkan-nouveau mesa-vulkan-virtio" ;;
		aarch64) apks="$apks mesa-vulkan-asahi mesa-vulkan-broadcom mesa-vulkan-freedreno mesa-vulkan-imagination mesa-vulkan-intel mesa-vulkan-nouveau mesa-vulkan-panfrost mesa-vulkan-virtio" ;;
	esac
	case "$ARCH" in
		x86|x86_64)
			syslinux_serial="0 115200"
			apks="$apks grub-bios amd-ucode intel-ucode"
			boot_addons="amd-ucode intel-ucode"
			initrd_ucode="/boot/amd-ucode.img /boot/intel-ucode.img"
			kernel_cmdline="$kernel_cmdline console=tty0 console=ttyS0,115200"
			;;
		ppc64le) apks="$(printf '%s' "$apks" | sed 's/grub-efi//g') grub-ieee1275" ;;
		s390x)
			apks="$(printf '%s' "$apks" | sed 's/grub-efi//g') s390-tools"
			kernel_cmdline="$kernel_cmdline console=ttysclp0"
			;;
		aarch64|armv7) kernel_cmdline="$kernel_cmdline console=tty0 console=ttyAMA0,115200" ;;
		riscv64) kernel_cmdline="$kernel_cmdline console=tty0 console=ttyS0,115200" ;;
	esac
}

# Change menu labels while retaining upstream kernel/initrd/firmware commands.
build_grub_cfg() {
	mkdir -p "$DESTDIR/$(dirname "$1")"
	grub_gen_config | sed 's/menuentry "Linux /menuentry "UFLinux /' > "$DESTDIR/$1"
}

build_syslinux_cfg() {
	mkdir -p "$DESTDIR/$(dirname "$1")"
	syslinux_gen_config | sed 's/MENU LABEL Linux /MENU LABEL UFLinux /' > "$DESTDIR/$1"
}
