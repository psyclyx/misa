// Browser-owned memory. The session supplies semantics; this page owns its draft,
// disclosure state and theme. Storage failures leave a usable in-memory client.
(function (global) {
  function memory(storage, key) {
    var saved;
    try {
      saved = JSON.parse(storage.getItem(key));
    } catch (_) {}
    saved = saved && typeof saved === "object" ? saved : {};
    var state = {
      theme: ["system", "dark", "plain"].includes(saved.theme)
        ? saved.theme
        : "system",
      opened: Array.isArray(saved.opened)
        ? saved.opened.filter(function (id) {
            return typeof id === "string";
          })
        : [],
      draft: typeof saved.draft === "string" ? saved.draft : "",
      frecency:
        saved.frecency && typeof saved.frecency === "object"
          ? saved.frecency
          : {},
    };
    return {
      state: state,
      save: function () {
        try {
          storage.setItem(key, JSON.stringify(state));
        } catch (_) {}
      },
    };
  }

  function start(doc, browser) {
    var main = doc.getElementById("main");
    if (!main) return;
    var storage;
    try {
      storage = browser.localStorage;
    } catch (_) {}
    var prefs = memory(
      storage,
      "misa.preferences." + (doc.body.dataset.session || "default"),
    );
    var theme = doc.getElementById("theme");
    function composer() {
      return main.querySelector('#composer textarea[name="prompt"]');
    }
    function restore() {
      doc.documentElement.dataset.theme = prefs.state.theme;
      if (theme) theme.value = prefs.state.theme;
      var draft = composer();
      if (draft) draft.value = prefs.state.draft;
      main.querySelectorAll("details[id]").forEach(function (node) {
        node.open =
          prefs.state.opened.includes(node.id) ||
          prefs.state.opened.includes("*");
      });
    }
    if (theme)
      theme.addEventListener("change", function () {
        prefs.state.theme = theme.value;
        prefs.save();
        restore();
      });
    main.addEventListener("input", function (event) {
      if (event.target === composer()) {
        prefs.state.draft = event.target.value;
        prefs.save();
      }
    });
    main.addEventListener(
      "toggle",
      function (event) {
        var node = event.target;
        if (node.tagName !== "DETAILS" || !node.id) return;
        var wasOpen = prefs.state.opened.includes(node.id);
        if (wasOpen === node.open) return;
        prefs.state.opened = prefs.state.opened.filter(function (id) {
          return id !== node.id;
        });
        if (node.open) prefs.state.opened.push(node.id);
        prefs.save();
      },
      true,
    );
    main.addEventListener("submit", function (event) {
      var draft = composer();
      if (draft && event.target.contains(draft)) {
        prefs.state.draft = "";
        prefs.save();
      }
    });
    restore();
    // Until a revision is represented by DOM ops, preserve browser-owned state
    // around a snapshot replacement. The same restoration applies after ops.
    function snapshot(html) {
      var draft = composer();
      var focused = draft && doc.activeElement === draft;
      var selection = focused
        ? [draft.selectionStart, draft.selectionEnd]
        : null;
      var following =
        browser.scrollY + browser.innerHeight >= doc.body.scrollHeight - 40;
      main.innerHTML = html;
      restoreStreams();
      restore();
      draft = composer();
      if (focused && draft) {
        draft.focus({ preventScroll: true });
        draft.setSelectionRange(selection[0], selection[1]);
      } else if (following) {
        browser.scrollTo({ top: doc.body.scrollHeight });
      }
    }
    function changes(ops) {
      function held(id) {
        var node = doc.getElementById(id);
        if (!node || !main.contains(node))
          throw Error("missing view node: " + id);
        return node;
      }
      function fragment(html) {
        var template = doc.createElement("template");
        template.innerHTML = html;
        if (template.content.childElementCount !== 1)
          throw Error("view operation needs one root");
        return template.content.firstElementChild;
      }
      ops.forEach(function (op) {
        if (op.op === "remove") held(op.id).remove();
        else if (op.op === "replace")
          held(op.id).replaceWith(fragment(op.html));
        else if (op.op === "insert") {
          var parent = held(op.parent);
          var before = op.before == null ? null : held(op.before);
          if (before && before.parentElement !== parent)
            throw Error("view insertion anchor belongs to another parent");
          parent.insertBefore(fragment(op.html), before);
        } else throw Error("unknown view operation: " + op.op);
      });
      restoreStreams();
      restore();
    }
    var activeStreams = new Map();
    function restoreStreams() {
      activeStreams.forEach(function (held, id) {
        var boundary = id.lastIndexOf(".");
        var owner = boundary < 0 ? id : id.slice(0, boundary);
        held.node.hidden = !!doc.getElementById(owner);
        if (!main.contains(held.node))
          (doc.getElementById("transcript") || main).appendChild(held.node);
      });
    }
    function streamUpdate(update) {
      if (update.update === "current") {
        var current = update.stream;
        streamUpdate({ update: "end", id: current.id });
        var node = doc.createElement("pre");
        node.dataset.stream = current.id;
        node.className = "stream";
        var text = doc.createTextNode(current.text);
        node.appendChild(text);
        (doc.getElementById("transcript") || main).appendChild(node);
        activeStreams.set(current.id, {
          node: node,
          text: text,
          bytes: new TextEncoder().encode(current.text).length,
        });
        restoreStreams();
      } else if (update.update === "append") {
        var held = activeStreams.get(update.id);
        if (!held || held.bytes !== update.offset)
          throw Error("stream append is missing its predecessor");
        held.text.appendData(update.text);
        held.bytes += new TextEncoder().encode(update.text).length;
      } else if (update.update === "end") {
        var ended = activeStreams.get(update.id);
        if (ended) ended.node.remove();
        activeStreams.delete(update.id);
      } else throw Error("unknown stream update");
    }
    function streams(current) {
      activeStreams.forEach(function (_, id) {
        streamUpdate({ update: "end", id: id });
      });
      current.forEach(function (stream) {
        streamUpdate({ update: "current", stream: stream });
      });
    }
    if (browser.EventSource) {
      var stream = new browser.EventSource("/events");
      stream.onmessage = function (message) {
        snapshot(message.data);
      };
      stream.addEventListener("changes", function (message) {
        changes(JSON.parse(message.data));
      });
      stream.addEventListener("streams", function (message) {
        streams(JSON.parse(message.data));
      });
      stream.addEventListener("stream", function (message) {
        streamUpdate(JSON.parse(message.data));
      });
    }
    return {
      prefs: prefs,
      snapshot: snapshot,
      changes: changes,
      streams: streams,
      streamUpdate: streamUpdate,
    };
  }
  global.MisaClient = { memory: memory, start: start };
  if (global.document) start(global.document, global);
})(globalThis);
