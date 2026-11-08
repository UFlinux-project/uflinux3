/* Test-only virtual pointer; coordinates are confined to the private Wayland session. */
#include <wayland-client.h>
#include "pointer-protocol.h"
#include <linux/input-event-codes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data, struct wl_registry *registry, uint32_t id, const char *name, uint32_t version) {
    (void)data; (void)version;
    if (!strcmp(name, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(registry,id,&zwlr_virtual_pointer_manager_v1_interface,1);
}
static void removed(void *data,struct wl_registry *registry,uint32_t id) {(void)data;(void)registry;(void)id;}
static uint32_t now(void) {struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return (uint32_t)(t.tv_sec*1000+t.tv_nsec/1000000);}
int main(int argc,char **argv) {
    if (argc!=7) {fputs("usage: pointer-driver x0 y0 x1 y1 width height\n",stderr);return 2;}
    uint32_t coordinates[6];
    for(int i=0;i<6;i++){char *end;long n=strtol(argv[i+1],&end,10);if(*end||n<0||n>100000)return 2;coordinates[i]=(uint32_t)n;}
    if(!coordinates[4]||!coordinates[5])return 2;
    struct wl_display *display=wl_display_connect(NULL);if(!display)return 1;
    struct wl_registry *registry=wl_display_get_registry(display);
    struct wl_registry_listener listener={global,removed};wl_registry_add_listener(registry,&listener,NULL);
    if(wl_display_roundtrip(display)<0||!manager)return 1;
    struct zwlr_virtual_pointer_v1 *pointer=zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager,NULL);
    zwlr_virtual_pointer_v1_motion_absolute(pointer,now(),coordinates[0],coordinates[1],coordinates[4],coordinates[5]);
    zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);usleep(150000);
    /* Activate the client before dragging; compositors may consume its first click. */
    zwlr_virtual_pointer_v1_button(pointer,now(),BTN_LEFT,WL_POINTER_BUTTON_STATE_PRESSED);
    zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);
    zwlr_virtual_pointer_v1_button(pointer,now(),BTN_LEFT,WL_POINTER_BUTTON_STATE_RELEASED);
    zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);usleep(150000);
    if(coordinates[0]==coordinates[2] && coordinates[1]==coordinates[3]) goto done;
    zwlr_virtual_pointer_v1_button(pointer,now(),BTN_LEFT,WL_POINTER_BUTTON_STATE_PRESSED);
    zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);
    for(int i=1;i<=10;i++) {
        int x=(int)coordinates[0]+((int)coordinates[2]-(int)coordinates[0])*i/10;
        int y=(int)coordinates[1]+((int)coordinates[3]-(int)coordinates[1])*i/10;
        zwlr_virtual_pointer_v1_motion_absolute(pointer,now(),x,y,coordinates[4],coordinates[5]);
        zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);usleep(50000);
    }
    zwlr_virtual_pointer_v1_button(pointer,now(),BTN_LEFT,WL_POINTER_BUTTON_STATE_RELEASED);
    zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);
done:
    usleep(150000);
    zwlr_virtual_pointer_v1_destroy(pointer);zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_registry_destroy(registry);wl_display_disconnect(display);return 0;
}
