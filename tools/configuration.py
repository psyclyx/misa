"""Build explicit Fennel applications for executable tests and benchmarks."""

import json


def fennel(value):
    """Encode JSON-shaped data as a Fennel expression."""
    if value is None:
        return "misa.json-null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, list):
        return "[" + " ".join(map(fennel, value)) + "]"
    return "{" + " ".join(fennel(k) + " " + fennel(v) for k, v in value.items()) + "}"


def application(settings, *, default=False, omit=()):
    """Compose a test application from explicit modules and settings."""
    modules = []
    for index, name in enumerate(settings.get("extensions", [])):
        path = "/" in name or name.endswith((".fnl", ".lua"))
        source = f"(fennel.dofile {fennel(name)})" if path else f"(require {fennel(name)})"
        modules.append(f'"test.{index}" {{:build {source} :priority {-100000 + index * 1000}}}')
    base = "standard.default" if default else "{}"
    module_expression = "{" + " ".join(modules) + "}"
    if omit:
        tests = " ".join(f'(= (id:sub 1 {len(prefix)}) {fennel(prefix)})' for prefix in omit)
        module_expression = (
            f'(let [modules {module_expression}] '
            f'(each [id (pairs standard.default.modules)] '
            f'(when (or {tests}) (tset modules id misa.delete))) modules)'
        )
    return (
        '(local standard (require :misa.standard))\n'
        '(local fennel (require :fennel))\n'
        f'(standard.application (misa.compose [{base}\n'
        f'{{:config {fennel(settings.get("config", {}))} :modules {module_expression}}}]))\n'
    )
