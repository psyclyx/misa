#include "shim.h"
#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int ref;
    char *path;
} misa_extension;

struct misa_lua {
    lua_State *state;
    int context_ref;
    misa_extension *extensions;
    size_t extension_count;
    char error[1024];
};

static const char *bootstrap =
    "local handlers = {}\n"
    "misa = {}\n"
    "function misa.register(name, handler)\n"
    "  assert(type(name) == 'string', 'handler name must be a string')\n"
    "  assert(type(handler) == 'function', 'handler must be a function')\n"
    "  local list = handlers[name] or {}; handlers[name] = list\n"
    "  list[#list + 1] = handler\n"
    "end\n"
    "function misa.call(name, ...)\n"
    "  local results = { n = 0 }\n"
    "  for _, handler in ipairs(handlers[name] or {}) do\n"
    "    results.n = results.n + 1\n"
    "    results[results.n] = handler(...)\n"
    "  end\n"
    "  return results\n"
    "end\n";

static int fail(misa_lua *runtime, const char *prefix) {
    const char *detail = lua_tostring(runtime->state, -1);
    snprintf(runtime->error, sizeof(runtime->error), "%s: %s", prefix, detail ? detail : "unknown Lua error");
    lua_pop(runtime->state, 1);
    return 0;
}

static int traceback(lua_State *L) {
    if (!lua_isstring(L, 1)) return 1;
    lua_getglobal(L, "debug");
    if (!lua_istable(L, -1)) { lua_pop(L, 1); return 1; }
    lua_getfield(L, -1, "traceback");
    if (!lua_isfunction(L, -1)) { lua_pop(L, 2); return 1; }
    lua_pushvalue(L, 1);
    lua_pushinteger(L, 2);
    lua_call(L, 2, 1);
    return 1;
}

misa_lua *misa_lua_new(void) {
    misa_lua *runtime = calloc(1, sizeof(*runtime));
    if (!runtime) return NULL;
    runtime->state = luaL_newstate();
    if (!runtime->state) { free(runtime); return NULL; }
    luaL_openlibs(runtime->state);
    runtime->context_ref = LUA_NOREF;
    if (luaL_loadstring(runtime->state, bootstrap) || lua_pcall(runtime->state, 0, 0, 0)) {
        fail(runtime, "initializing misa API");
        misa_lua_free(runtime);
        return NULL;
    }
    return runtime;
}

void misa_lua_free(misa_lua *runtime) {
    if (!runtime) return;
    if (runtime->state) lua_close(runtime->state);
    for (size_t i = 0; i < runtime->extension_count; ++i) free(runtime->extensions[i].path);
    free(runtime->extensions);
    free(runtime);
}

int misa_lua_set_context(misa_lua *runtime, const char *config_json, int argc, const char *const *argv) {
    lua_State *L = runtime->state;
    lua_newtable(L);
    lua_pushstring(L, config_json);
    lua_setfield(L, -2, "config_json");
    lua_newtable(L);
    for (int i = 0; i < argc; ++i) {
        lua_pushstring(L, argv[i]);
        lua_rawseti(L, -2, i + 1);
    }
    lua_setfield(L, -2, "argv");
    lua_getglobal(L, "misa");
    lua_setfield(L, -2, "misa");
    runtime->context_ref = luaL_ref(L, LUA_REGISTRYINDEX);
    return 1;
}

int misa_lua_load_extension(misa_lua *runtime, const char *path) {
    lua_State *L = runtime->state;
    if (luaL_loadfile(L, path)) return fail(runtime, path);
    if (lua_pcall(L, 0, 1, 0)) return fail(runtime, path);
    if (!lua_istable(L, -1)) {
        snprintf(runtime->error, sizeof(runtime->error), "%s: extension must return a table", path);
        lua_pop(L, 1);
        return 0;
    }
    char *path_copy = malloc(strlen(path) + 1);
    misa_extension *next = realloc(runtime->extensions, sizeof(*next) * (runtime->extension_count + 1));
    if (!path_copy || !next) {
        free(path_copy);
        if (next) runtime->extensions = next;
        snprintf(runtime->error, sizeof(runtime->error), "out of memory while loading %s", path);
        lua_pop(L, 1);
        return 0;
    }
    strcpy(path_copy, path);
    runtime->extensions = next;
    runtime->extensions[runtime->extension_count].ref = luaL_ref(L, LUA_REGISTRYINDEX);
    runtime->extensions[runtime->extension_count].path = path_copy;
    runtime->extension_count++;
    return 1;
}

static int call_phase(misa_lua *runtime, const char *phase) {
    lua_State *L = runtime->state;
    for (size_t i = 0; i < runtime->extension_count; ++i) {
        const misa_extension *extension = &runtime->extensions[i];
        lua_rawgeti(L, LUA_REGISTRYINDEX, extension->ref);
        lua_pushstring(L, phase);
        lua_rawget(L, -2);
        if (lua_isnil(L, -1)) { lua_pop(L, 2); continue; }
        if (!lua_isfunction(L, -1)) {
            snprintf(runtime->error, sizeof(runtime->error), "extension %zu (%s) field '%s' must be a function", i + 1, extension->path, phase);
            lua_pop(L, 2);
            return 0;
        }

        lua_pushcfunction(L, traceback);
        lua_insert(L, -2);
        int error_handler = lua_gettop(L) - 1;
        lua_rawgeti(L, LUA_REGISTRYINDEX, runtime->context_ref);
        if (lua_pcall(L, 1, 0, error_handler)) {
            const char *detail = lua_tostring(L, -1);
            snprintf(runtime->error, sizeof(runtime->error), "extension %zu (%s) %s: %s", i + 1, extension->path, phase, detail ? detail : "unknown Lua error");
            lua_settop(L, error_handler - 2);
            return 0;
        }
        lua_pop(L, 2);
    }
    return 1;
}

int misa_lua_run(misa_lua *runtime) {
    return call_phase(runtime, "setup") && call_phase(runtime, "run");
}

const char *misa_lua_error(const misa_lua *runtime) { return runtime->error; }
