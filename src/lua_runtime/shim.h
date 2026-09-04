#ifndef MISA_LUA_SHIM_H
#define MISA_LUA_SHIM_H

typedef struct misa_lua misa_lua;
misa_lua *misa_lua_new(void);
void misa_lua_free(misa_lua *runtime);
int misa_lua_set_context(misa_lua *runtime, const char *config_json, int argc, const char *const *argv);
int misa_lua_load_extension(misa_lua *runtime, const char *path);
int misa_lua_run(misa_lua *runtime);
const char *misa_lua_error(const misa_lua *runtime);

#endif
