/* A short-lived hardware probe: never pick lavapipe/llvmpipe automatically. */
#include "renderer_ids.h"
#include <vulkan/vulkan.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdlib.h>
#include <string.h>

/* A DRM node alone does not prove acceleration (e.g. virtio without virgl).
 * Initialize the device before deciding; Mesa can fall back to llvmpipe. */
static int accelerated_gl(EGLDeviceEXT device) {
    PFNEGLGETPLATFORMDISPLAYEXTPROC platform = (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!platform) return 0;
    EGLDisplay display = platform(EGL_PLATFORM_DEVICE_EXT, device, NULL);
    if (display == EGL_NO_DISPLAY || !eglInitialize(display, NULL, NULL)) return 0;
    EGLConfig config; EGLint count = 0;
    const EGLint attributes[] = {EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE};
    const EGLint context_attributes[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
    const EGLint surface_attributes[] = {EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE};
    EGLContext context = EGL_NO_CONTEXT;
    EGLSurface surface = EGL_NO_SURFACE;
    int accelerated = 0;
    if (eglBindAPI(EGL_OPENGL_ES_API) && eglChooseConfig(display, attributes, &config, 1, &count) && count) {
        context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_attributes);
        surface = eglCreatePbufferSurface(display, config, surface_attributes);
        if (context != EGL_NO_CONTEXT && surface != EGL_NO_SURFACE && eglMakeCurrent(display, surface, surface, context)) {
            const char *renderer = (const char *)glGetString(GL_RENDERER);
            accelerated = renderer && !strstr(renderer, "llvmpipe") && !strstr(renderer, "softpipe") &&
                !strstr(renderer, "Software") && !strstr(renderer, "SWR") && !strstr(renderer, "lavapipe");
            eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        }
    }
    if (surface != EGL_NO_SURFACE) eglDestroySurface(display, surface);
    if (context != EGL_NO_CONTEXT) eglDestroyContext(display, context);
    eglTerminate(display);
    return accelerated;
}

int uflinux_renderer_probe(void) {
    VkInstance instance;
    VkApplicationInfo app = {.sType=VK_STRUCTURE_TYPE_APPLICATION_INFO, .apiVersion=VK_API_VERSION_1_1};
    VkInstanceCreateInfo info = {.sType=VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, .pApplicationInfo=&app};
    if (vkCreateInstance(&info, NULL, &instance) == VK_SUCCESS) {
        uint32_t count = 0;
        if (vkEnumeratePhysicalDevices(instance, &count, NULL) == VK_SUCCESS && count && count < 64) {
            VkPhysicalDevice devices[64];
            if (vkEnumeratePhysicalDevices(instance, &count, devices) == VK_SUCCESS) {
                for (uint32_t i=0; i<count; ++i) {
                    VkPhysicalDeviceProperties p;
                    vkGetPhysicalDeviceProperties(devices[i], &p);
                    if ((p.deviceType==VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU ||
                         p.deviceType==VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU) &&
                        !uflinux_legacy_intel_gpu(p.vendorID, p.deviceID)) {
                        vkDestroyInstance(instance, NULL);
                        return 2;
                    }
                }
            }
        }
        vkDestroyInstance(instance, NULL);
    }
    PFNEGLQUERYDEVICESEXTPROC query = (void *)eglGetProcAddress("eglQueryDevicesEXT");
    PFNEGLQUERYDEVICESTRINGEXTPROC text = (void *)eglGetProcAddress("eglQueryDeviceStringEXT");
    if (query && text) {
        EGLDeviceEXT devices[32]; EGLint count=0;
        if (query(32, devices, &count)) {
            for (int i=0; i<count; ++i) {
                const char *ext = text(devices[i], EGL_EXTENSIONS);
                if (ext && !strstr(ext, "EGL_MESA_device_software") &&
                    (strstr(ext, "EGL_EXT_device_drm") || strstr(ext, "EGL_EXT_device_drm_render_node")) &&
                    accelerated_gl(devices[i])) return 1;
            }
        }
    }
    return 0;
}
