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
	initfs_features="$initfs_features kms xfs uflinux-plymouth"
	# The diskless live root includes Alpine's full firmware collection.
	initfs_cmdline="$initfs_cmdline rootflags=mode=0755,size=75%"
	modloop_addons="$modloop_addons uflinux-initramfs"
	apks="$apks uflinux uflinux-live uflinux-installer linux-lts linux-firmware
		systemd-boot xfsprogs e2fsprogs dosfstools util-linux vulkan-loader mesa-vulkan-ati"
	case "$ARCH" in
		x86|x86_64|aarch64|armv7|riscv64) apks="$apks efibootmgr" ;;
	esac
	case "$ARCH" in
		x86|x86_64) apks="$apks mesa-vulkan-intel mesa-vulkan-nouveau mesa-vulkan-virtio" ;;
		aarch64) apks="$apks mesa-vulkan-asahi mesa-vulkan-broadcom mesa-vulkan-freedreno mesa-vulkan-imagination mesa-vulkan-intel mesa-vulkan-nouveau mesa-vulkan-panfrost mesa-vulkan-virtio" ;;
	esac
	case "$ARCH" in
		x86|x86_64)
			syslinux_serial="0 115200"
			apks="$apks syslinux amd-ucode intel-ucode"
			boot_addons="amd-ucode intel-ucode"
			initrd_ucode="/boot/amd-ucode.img /boot/intel-ucode.img"
			kernel_cmdline="$kernel_cmdline console=tty0 console=ttyS0,115200"
			;;
		ppc64le) apks="$(printf '%s' "$apks" | sed 's/systemd-boot//g') grub-ieee1275" ;;
		s390x)
			apks="$(printf '%s' "$apks" | sed 's/systemd-boot//g') s390-tools"
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

# systemd-boot loads its entries and kernel from the same EFI filesystem.
# The ISO therefore includes a FAT EFI image with its own kernel and initramfs.
section_grub_efi () {
	local suffix
	case "$ARCH" in
		x86) suffix=ia32 ;;
		x86_64) suffix=x64 ;;
		aarch64) suffix=aa64 ;;
		armv7) suffix=arm ;;
		riscv64) suffix=riscv64 ;;
		*) return 0 ;;
	esac
	build_section systemd_efi "$suffix" "$(apk fetch --root "$APKROOT" --simulate systemd-boot | checksum)"
}

build_systemd_efi() {
	local suffix="$1" binary="usr/lib/systemd/boot/efi/systemd-boot$1.efi"
	apk fetch --root "$APKROOT" --quiet --stdout systemd-boot | tar -C "$WORKDIR" -zx "$binary" || return 1
	mkdir -p "$DESTDIR/efi/boot"
	cp "$WORKDIR/$binary" "$DESTDIR/efi/boot/boot${suffix}.efi"
}

create_image_iso() {
	local ISO="${OUTDIR}/${output_filename}" _isolinux="" _efiboot="" _f _p image sectors size
	local flavors="${kernel_flavors-}"
	if [ -e "$DESTDIR/boot/syslinux/isolinux.bin" ]; then
		_isolinux="-isohybrid-mbr $DESTDIR/boot/syslinux/isohdpfx.bin -eltorito-boot boot/syslinux/isolinux.bin -eltorito-catalog boot/syslinux/boot.cat -no-emul-boot -boot-load-size 4 -boot-info-table"
	fi
	if [ -d "$DESTDIR/efi/boot" ]; then
		mkdir -p "$WORKDIR/systemd-loader/loader/entries" "$DESTDIR/boot"
		printf 'default uflinux-%s.conf\ntimeout 1\n' "${flavors%% *}" > "$WORKDIR/systemd-loader/loader/loader.conf"
		for _f in $flavors; do
			{
				printf 'title UFLinux %s\nlinux /boot/vmlinuz-%s\n' "$_f" "$_f"
				for _p in $initrd_ucode; do printf 'initrd %s\n' "$_p"; done
				printf 'initrd /boot/initramfs-%s\noptions %s %s\n' "$_f" "${initfs_cmdline-}" "$kernel_cmdline"
			} > "$WORKDIR/systemd-loader/loader/entries/uflinux-$_f.conf"
		done
		# FAT32 needs adequate cluster overhead, including for microcode images.
		size=$((64 * 1024 * 1024))
		for _f in $flavors; do
			size=$((size + $(wc -c < "$DESTDIR/boot/vmlinuz-$_f") + $(wc -c < "$DESTDIR/boot/initramfs-$_f")))
		done
		for _p in $initrd_ucode; do size=$((size + $(wc -c < "$DESTDIR$_p"))); done
		sectors=$(((size + 511) / 512))
		image="$DESTDIR/boot/efi.img"
		mformat -i "$image" -C -T "$sectors" -F :: || return 1
		mcopy -i "$image" -s "$DESTDIR/efi" "$WORKDIR/systemd-loader/loader" :: || return 1
		mmd -i "$image" ::/boot || return 1
		for _f in $flavors; do
			mcopy -i "$image" "$DESTDIR/boot/vmlinuz-$_f" "$DESTDIR/boot/initramfs-$_f" ::/boot/ || return 1
		done
		for _p in $initrd_ucode; do mcopy -i "$image" "$DESTDIR$_p" ::/boot/ || return 1; done
		touch -md "@$SOURCE_DATE_EPOCH" "$image"
		if [ -z "$_isolinux" ]; then
			_efiboot="-efi-boot-part --efi-boot-image -e boot/efi.img -no-emul-boot"
		else
			_efiboot="-eltorito-alt-boot -e boot/efi.img -no-emul-boot -isohybrid-gpt-basdat"
		fi
	fi
	if [ "$ARCH" = ppc64le ]; then
		apk fetch --root "$APKROOT" --quiet --stdout grub-ieee1275 | tar -C "$WORKDIR" -zx usr/lib/grub
		grub-mkrescue --output "$ISO" "$DESTDIR" -follow-links --directory="$WORKDIR/usr/lib/grub/powerpc-ieee1275" -sysid LINUX -volid "$(gen_volid)"
	else
		if [ "$ARCH" = s390x ]; then
			printf %s "$initfs_cmdline $kernel_cmdline " > "$WORKDIR/parmfile"
			for _f in $flavors; do
				mk-s390image -p "$WORKDIR/parmfile" -r "$DESTDIR/boot/initramfs-$_f" "$DESTDIR/boot/vmlinuz-$_f" "$DESTDIR/boot/merged.img"
			done
			iso_opts="$iso_opts -no-emul-boot -eltorito-boot boot/merged.img"
		fi
		# xorriso option variables intentionally expand as multiple arguments.
		# shellcheck disable=SC2086
		xorrisofs -quiet -output "$ISO" -full-iso9660-filenames -joliet -rational-rock -sysid LINUX -volid "$(gen_volid)" $_isolinux $_efiboot -follow-links ${iso_opts} "$DESTDIR"
	fi
}
