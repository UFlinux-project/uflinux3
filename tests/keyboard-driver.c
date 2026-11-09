/* Persistent virtual keyboard for real GTK/Wayland interaction tests. */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include "keyboard-protocol.h"

static struct wl_seat *seat;
static struct zwp_virtual_keyboard_manager_v1 *manager;
static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    (void)data; (void)version;
    if (!strcmp(interface, wl_seat_interface.name) && !seat)
        seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
    if (!strcmp(interface, zwp_virtual_keyboard_manager_v1_interface.name))
        manager = wl_registry_bind(registry, name, &zwp_virtual_keyboard_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static uint32_t milliseconds(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now)) abort();
    return (uint32_t)((uint64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000);
}
int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 1;
    struct wl_registry *registry = wl_display_get_registry(display);
    const struct wl_registry_listener listener = {global, removed};
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !seat || !manager) return 1;
    struct xkb_context *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    // Single-level symbols match wtype without the physical PC layout's
    // Ctrl+Alt+Fn -> VT-switch translations overriding compositor test binds.
    const char definition[] =
        "xkb_keymap { xkb_keycodes { minimum=8; maximum=255; "
        "<ESC>=9; <RET>=36; <DOWN>=116; <UP>=111; <I>=31; "
        "<LWIN>=133; <RWIN>=134; <F10>=76; <F11>=95; }; "
        "xkb_types { include \"complete\" }; "
        "xkb_compatibility { include \"complete\" }; "
        "xkb_symbols { key <ESC> {[ Escape ]}; key <RET> {[ Return ]}; "
        "key <DOWN> {[ Down ]}; key <UP> {[ Up ]}; key <I> {[ i ]}; "
        "key <LWIN> {[ Super_L ]}; key <RWIN> {[ Super_R ]}; "
        "key <F10> {[ F10 ]}; key <F11> {[ F11 ]}; }; };";
    struct xkb_keymap *map = xkb_keymap_new_from_string(context, definition,
        XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
    if (!map) return 1;
    char *keymap = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
    FILE *file = tmpfile();
    if (!file || !keymap) return 1;
    size_t size = strlen(keymap) + 1;
    if (fwrite(keymap, 1, size, file) != size || fflush(file)) return 1;
    struct zwp_virtual_keyboard_v1 *keyboard = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(manager, seat);
    zwp_virtual_keyboard_v1_keymap(keyboard, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fileno(file), (uint32_t)size);
    if (wl_display_roundtrip(display) < 0) return 1;
    fclose(file); free(keymap);
    setvbuf(stdout, NULL, _IOLBF, 0);
    puts("ready");
    char line[128], operation[16], value[64];
    uint32_t manual_mods = 0;
    while (fgets(line, sizeof(line), stdin)) {
        int down = 0;
        if (!strcmp(line, "quit\n")) break;
        int count = sscanf(line, "%15s %63s %d", operation, value, &down);
        if (count < 2) return 2;
        if (!strcmp(operation, "mod")) {
            const char *name = !strcmp(value, "logo") ? XKB_MOD_NAME_LOGO :
                               !strcmp(value, "ctrl") ? XKB_MOD_NAME_CTRL :
                               !strcmp(value, "alt") ? XKB_MOD_NAME_ALT : NULL;
            if (!name || count != 3) return 2;
            xkb_mod_index_t index = xkb_keymap_mod_get_index(map, name);
            if (index == XKB_MOD_INVALID || index >= 32) return 2;
            if (down) manual_mods |= 1u << index; else manual_mods &= ~(1u << index);
            zwp_virtual_keyboard_v1_modifiers(keyboard, manual_mods, 0, 0, 0);
        } else if (!strcmp(operation, "key")) {
            xkb_keysym_t symbol = xkb_keysym_from_name(value, XKB_KEYSYM_NO_FLAGS);
            xkb_keycode_t code;
            for (code = xkb_keymap_min_keycode(map); code <= xkb_keymap_max_keycode(map); ++code) {
                const xkb_keysym_t *symbols;
                int n = xkb_keymap_key_get_syms_by_level(map, code, 0, 0, &symbols);
                if (n == 1 && symbols[0] == symbol) break;
            }
            if (symbol == XKB_KEY_NoSymbol || code > xkb_keymap_max_keycode(map) || code < 8) return 2;
            for (int pressed = 1; pressed >= 0; --pressed) {
                zwp_virtual_keyboard_v1_key(keyboard, milliseconds(), code - 8, (uint32_t)pressed);
                if (wl_display_roundtrip(display) < 0) return 1;
            }
        } else return 2;
        if (wl_display_roundtrip(display) < 0) return 1;
        puts("ok");
    }
    zwp_virtual_keyboard_v1_destroy(keyboard);
    wl_display_flush(display);
    xkb_keymap_unref(map); xkb_context_unref(context);
    wl_display_disconnect(display);
    return 0;
}
