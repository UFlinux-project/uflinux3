#!/bin/sh
set -eu
hostname=${1:?hostname is required}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/etc/apk" "$tmp/etc/runlevels"
printf '%s\n' "$hostname" > "$tmp/etc/hostname"
case "${ARCH:?}" in
	armv7|aarch64) serial=ttyAMA0 ;;
	ppc64le) serial=hvc0 ;;
	s390x) serial=ttysclp0 ;;
	*) serial=ttyS0 ;;
esac
cat > "$tmp/etc/inittab" <<EOF
::sysinit:/sbin/openrc sysinit
::sysinit:/sbin/openrc boot
::wait:/sbin/openrc default
tty2::respawn:/sbin/getty 38400 tty2
$serial::respawn:/sbin/getty -L 115200 $serial vt100
::ctrlaltdel:/sbin/reboot
::shutdown:/sbin/openrc shutdown
EOF
printf '%s\n' alpine-base uflinux uflinux-live uflinux-installer > "$tmp/etc/apk/world"
printf '%s\n' "${UFLINUX_REPOSITORY_URL:?}" \
	'https://dl-cdn.alpinelinux.org/alpine/v3.24/main' \
	'https://dl-cdn.alpinelinux.org/alpine/v3.24/community' > "$tmp/etc/apk/repositories"
# The signed ISO boot repository is discovered by Alpine's initramfs.
# Installed repository URLs are supplied by uflinux-release.
for entry in \
	'devfs sysinit' 'dmesg sysinit' 'udev sysinit' 'udev-trigger sysinit' \
	'udev-settle sysinit' 'hwdrivers sysinit' 'modloop sysinit' \
	'uflinux-splash sysinit' 'uflinux-splash-stop default' 'hwclock boot' 'modules boot' 'sysctl boot' 'hostname boot' \
	'bootmisc boot' 'syslog boot' 'dbus default' 'elogind default' \
	'iwd default' 'connman default' 'ntpd default' 'tlp default' 'uflinux-acpid default' 'uflinux-routes default' \
	'uflinux-live default' 'greetd default' \
	'killprocs shutdown' 'mount-ro shutdown'; do
	# Intentional service/runlevel pair splitting.
	# shellcheck disable=SC2086
	set -- $entry
	mkdir -p "$tmp/etc/runlevels/$2"
	ln -s "/etc/init.d/$1" "$tmp/etc/runlevels/$2/$1"
done
tar --sort=name --mtime="@${SOURCE_DATE_EPOCH:?}" --owner=0 --group=0 --numeric-owner \
	-C "$tmp" -cf - etc | gzip -9n > "$hostname.apkovl.tar.gz"
