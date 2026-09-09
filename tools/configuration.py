"""Write explicit application data for executable tests and benchmarks."""

import json
from pathlib import Path


def fennel(value):
    """Encode JSON-shaped fixture data as a Fennel expression."""
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
    """Assemble fixture catalogs with explicit replacement and event order."""
    root = Path(__file__).resolve().parents[1]
    search_path = f"{root}/?.fnl;{root}/?/init.fnl;"
    base = "(misa.snapshot (require :misa.standard))" if default else "{:config {} :definitions {}}"
    lines = [
        '(local fennel (require :fennel))',
        f'(set fennel.path (.. {fennel(search_path)} fennel.path))',
        f'(local application {base})',
        f'(set application.config (misa.patch application.config {fennel(settings.get("config", {}))}))',
        '(local context {:config application.config :argv []})',
    ]
    if omit or any("/" not in name and not name.endswith((".fnl", ".lua"))
                   for name in settings.get("extensions", [])):
        lines.append('(local stock (require :tests.stock))')
    if omit:
        matches = " ".join(
            f'(= (name:sub 1 {len(prefix)}) {fennel(prefix)})' for prefix in omit
        )
        lines.append(
            '(each [name catalogs (pairs stock)] '
            f'(when (or {matches}) '
            '(each [kind entries (pairs catalogs)] '
            '(when (. application.definitions kind) '
            '(each [id (pairs entries)] (tset (. application.definitions kind) id nil))))))'
        )
    for index, name in enumerate(settings.get("extensions", [])):
        path = "/" in name or name.endswith((".fnl", ".lua"))
        if path:
            loader = "fennel.dofile" if name.endswith(".fnl") else "dofile"
            source = f'(({loader} {fennel(name)}) context)'
        else:
            source = f'(assert (. stock {fennel(name)}) "unknown fixture catalog")'
        lines.append(
            f'(let [catalogs {source}] '
            '(each [kind entries (pairs catalogs)] '
            '(when (not (. application.definitions kind)) (tset application.definitions kind {})) '
            '(each [id value (pairs entries)] '
            '(let [entry (misa.snapshot value)] '
            f'(when (= kind :events) (set entry.priority {index * 1000 - 100000})) '
            '(tset (. application.definitions kind) id entry)))))'
        )
    lines.append('application')
    return "\n".join(lines) + "\n"
