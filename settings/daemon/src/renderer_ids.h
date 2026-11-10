/* PCI IDs for Sandy Bridge, Ivy Bridge and Bay Trail graphics. */
static int uflinux_legacy_intel_gpu(unsigned int vendor, unsigned int device) {
    if (vendor != 0x8086) return 0;
    switch (device) {
    case 0x0102: case 0x0106: case 0x010a:
    case 0x0112: case 0x0116: case 0x0122: case 0x0126:
    case 0x0152: case 0x0156: case 0x015a:
    case 0x0162: case 0x0166: case 0x016a:
    case 0x0f30: case 0x0f31: case 0x0f32: case 0x0f33:
        return 1;
    default:
        return 0;
    }
}
