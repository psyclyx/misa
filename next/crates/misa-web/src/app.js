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
      recoveries: Array.isArray(saved.recoveries) ? saved.recoveries.filter(function (entry) {
        return entry && typeof entry.id === "string" && typeof entry.text === "string" && typeof entry.reason === "string";
      }).slice(0, 16) : [],
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
    if (browser.location && /^\/view\/[^/]+\/$/.test(browser.location.pathname) && doc.body.dataset.session) {
      try { browser.localStorage.setItem("misa.view." + browser.location.pathname.split("/")[2], doc.body.dataset.session); } catch (_) {}
    }
    if (browser.fetch && browser.location && /^\/view\/[^/]+\/$/.test(browser.location.pathname) && !doc.body.dataset.claimed) {
      doc.body.inert = true;
      browser.fetch("./claim", { method: "POST" }).then(async function (response) {
        if (!response.ok) throw Error(await response.text());
        var claim = await response.json();
        if (claim.url !== browser.location.pathname) {
          // A copied/reloaded document gets independent local memory. Copying
          // its initial draft does not link future edits between instances.
          try {
            var previous = browser.localStorage.getItem("misa.preferences." + doc.body.dataset.session);
            if (previous && claim.memory) browser.localStorage.setItem("misa.preferences." + claim.memory, previous);
          } catch (_) {}
          browser.location.replace(claim.url);
          return;
        }
        doc.body.dataset.claimed = "true";
        doc.body.inert = false;
        start(doc, browser);
      }).catch(function (error) {
        doc.body.inert = false;
        doc.querySelectorAll("form").forEach(function (form) { form.inert = true; });
        main.replaceChildren();
        var notice = doc.createElement("p"); notice.setAttribute("role", "alert");
        notice.textContent = error.message || "Unable to open an independent presentation";
        main.appendChild(notice);
      });
      return;
    }
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
    var draftVersion = 0;
    main.addEventListener("input", function (event) {
      if (event.target === composer()) {
        draftVersion++;
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
    var pendingForms = new Set();
    var submission = 0;
    function recoveryPanel() {
      var held = doc.getElementById("misa-recoveries");
      if (held) held.remove();
      if (!prefs.state.recoveries.length) return;
      var panel = doc.createElement("details");
      panel.id = "misa-recoveries";
      var title = doc.createElement("summary");
      title.textContent = "Pending or unsent submissions (" + prefs.state.recoveries.length + ")";
      panel.appendChild(title);
      prefs.state.recoveries.forEach(function (entry) {
        var section = doc.createElement("section");
        var reason = doc.createElement("p");
        reason.textContent = entry.reason;
        var text = doc.createElement("textarea");
        text.readOnly = true;
        text.setAttribute("aria-label", "Submission text to recover");
        text.value = entry.text;
        var discard = doc.createElement("button");
        discard.type = "button";
        discard.textContent = "Dismiss recovery copy";
        discard.addEventListener("click", function () {
          prefs.state.recoveries = prefs.state.recoveries.filter(function (candidate) { return candidate.id !== entry.id; });
          prefs.save();
          recoveryPanel();
        });
        section.append(reason, text, discard);
        panel.appendChild(section);
      });
      main.before(panel);
    }
    var preparationDrafts = new WeakMap();
    function report(html, preparation) {
      var dialog = doc.createElement("dialog");
      if (preparation) preparationDrafts.set(dialog, preparation);
      var content = doc.createElement("section");
      content.innerHTML = html;
      var close = doc.createElement("button");
      close.type = "button";
      close.textContent = "Close report";
      function dismiss() { dialog.close(); dialog.remove(); }
      close.addEventListener("click", dismiss);
      dialog.addEventListener("cancel", function (event) { event.preventDefault(); dismiss(); });
      dialog.addEventListener("close", function () { dialog.remove(); });
      dialog.append(content, close);
      doc.body.appendChild(dialog);
      dialog.showModal();
    }
    doc.addEventListener("submit", async function (event) {
      var form = event.target;
      // A control named "action" shadows HTMLFormElement.action. Read the DOM
      // attribute, not named form properties, for the transport destination.
      var destination = new URL(form.getAttribute("action") || "", doc.baseURI).href;
      if (!browser.fetch || (!destination.endsWith("/intent") && !destination.endsWith("/perform"))) return;
      event.preventDefault();
      var key = form.getAttribute("id") || destination;
      if (pendingForms.has(key) || pendingForms.size >= 16) return;
      var draft = composer();
      var isComposer = draft && form.contains(draft);
      if (isComposer && prefs.state.recoveries.length >= 16) {
        submissionStatus("Resolve a saved submission before sending another.");
        return;
      }
      var version = draftVersion;
      var saved = isComposer ? { id: String(Date.now()) + ":" + (++submission), text: draft.value, reason: "Reply not confirmed. Check the session before retrying." } : null;
      if (saved) { prefs.state.recoveries.push(saved); prefs.save(); recoveryPanel(); }
      pendingForms.add(key);
      form.setAttribute("aria-busy", "true");
      try {
        var fields = new FormData(form, event.submitter || undefined);
        var response = await browser.fetch(destination, { method: "POST", headers: { "Accept": "application/json" }, body: new URLSearchParams(fields) });
        var result = await response.json();
        if (!response.ok || result.ok !== true) throw Error(result.error || "Submission was not confirmed");
        if (saved) {
          prefs.state.recoveries = prefs.state.recoveries.filter(function (entry) { return entry.id !== saved.id; });
          if (draftVersion === version && !result.preparation) {
            prefs.state.draft = "";
            var current = composer();
            if (current) current.value = "";
          }
          prefs.save();
          recoveryPanel();
        }
        if (form.closest("dialog")) {
          var dialog = form.closest("dialog"), preparation = preparationDrafts.get(dialog);
          if (preparation && draftVersion === preparation.version) {
            prefs.state.draft = ""; var current = composer(); if (current) current.value = ""; prefs.save();
          }
          dialog.close(); dialog.remove();
        }
        if (result.report) report(result.report, result.preparation ? { version: version } : null);
        submissionStatus("");
      } catch (error) {
        var reason = (error.message || "Reply lost; execution may have occurred.") + " Check the session before retrying.";
        if (saved) { saved.reason = reason; prefs.save(); recoveryPanel(); }
        submissionStatus(reason);
      } finally { pendingForms.delete(key); form.removeAttribute("aria-busy"); }
    });
    recoveryPanel();
    restore();
    // Defaults for future instances are browser-owned. Live views keep their
    // own selections; changing this profile does not reconfigure other tabs.
    var presentationKey = "misa.presentations." + (doc.body.dataset.preferences || doc.body.dataset.session || "default");
    var presentationChoices = new Map();
    try {
      var savedChoices = JSON.parse(storage.getItem(presentationKey));
      if (savedChoices && typeof savedChoices === "object") Object.entries(savedChoices).slice(0, 128).forEach(function (entry) {
        if (typeof entry[1] === "string") presentationChoices.set(entry[0], entry[1]);
      });
    } catch (_) {}
    var presentationWork = Promise.resolve();
    var pendingPresentations = new Set();
    function configurePresentation(form, choice) {
      if (pendingPresentations.has(form)) return;
      if (pendingPresentations.size >= 128) { submissionStatus("Too many pending presentation changes"); return; }
      pendingPresentations.add(form);
      var select = form.elements.choice;
      var button = form.querySelector("button");
      select.disabled = true; button.disabled = true;
      presentationWork = presentationWork.then(async function () {
        try {
          var fields = new URLSearchParams({ id: form.elements.id.value, choice: choice });
          var response = await browser.fetch(form.action, { method: "POST", headers: { "Accept": "application/json" }, body: fields });
          var result = await response.json();
          if (!response.ok || result.ok !== true) throw Error(result.error || "Presentation change was not confirmed");
          select.value = choice;
          form.dataset.appliedChoice = choice;
          presentationChoices.set(form.elements.id.value, choice);
          try { storage.setItem(presentationKey, JSON.stringify(Object.fromEntries(presentationChoices))); } catch (_) {}
          submissionStatus("");
        } catch (error) { select.value = form.dataset.appliedChoice; submissionStatus(error.message || "Presentation change failed"); }
        finally { pendingPresentations.delete(form); select.disabled = false; button.disabled = false; }
      });
    }
    doc.querySelectorAll("form[data-presentation-choice]").forEach(function (form) {
      if (!browser.fetch) return;
      form.dataset.appliedChoice = form.elements.choice.value;
      form.addEventListener("submit", function (event) { event.preventDefault(); configurePresentation(form, form.elements.choice.value); });
      var savedChoice = presentationChoices.get(form.elements.id.value);
      if (savedChoice && savedChoice !== form.elements.choice.value) {
        if (Array.from(form.elements.choice.options).some(function (option) { return option.value === savedChoice; })) configurePresentation(form, savedChoice);
        else submissionStatus("A saved presentation variant is unavailable; choose a supported variant in Presentations.");
      }
    });
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
    var documentStreams = new Map();
    var documentPrefix = "";
    function restoreStreams() {
      activeStreams.forEach(function (held, id) {
        var boundary = id.lastIndexOf(".");
        var owner = boundary < 0 ? id : id.slice(0, boundary);
        held.node.hidden = !!doc.getElementById(documentPrefix + owner);
        if (!main.contains(held.node))
          (doc.getElementById(documentPrefix + "transcript") || main).appendChild(held.node);
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
        (doc.getElementById(documentPrefix + "transcript") || main).appendChild(node);
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
    // One SSE event is one owner publication. JavaScript applies it in a single
    // task, so paint and MutationObserver delivery cannot see a half publication.
    var invalid = false;
    function connectionStatus(text) {
      notice("misa-connection-status", text);
    }
    function submissionStatus(text) {
      notice("misa-submission-status", text);
    }
    function notice(id, text) {
      var notice = doc.getElementById(id);
      if (!notice) {
        notice = doc.createElement("p");
        notice.id = id;
        notice.setAttribute("role", "status");
        main.before(notice);
      }
      notice.textContent = text;
      notice.hidden = !text;
    }
    var recoveryFocus = null;
    var presentationClosed = false, eventStream = null;
    function invalidate() {
      var draft = composer();
      if (!invalid && draft && doc.activeElement === draft)
        recoveryFocus = [draft.selectionStart, draft.selectionEnd];
      invalid = true;
      main.hidden = true;
    }
    function transaction(messages) {
      if (presentationClosed) return;
      var closed = messages.find(function (message) { return message.kind === "closed"; });
      if (closed) {
        presentationClosed = true;
        if (eventStream) eventStream.close();
        connectionStatus(closed.data);
        doc.querySelectorAll("form button, form input, form select, form textarea").forEach(function (control) { control.disabled = true; });
        return;
      }
      try {
        // Work summaries are an independent observation. They neither repair
        // nor invalidate the selected document replica.
        messages = messages.filter(function (message) {
          if (message.kind !== "activity") return true;
          var activity = doc.getElementById("activity");
          if (activity) activity.innerHTML = message.data;
          return false;
        });
        if (!messages.length) return;
        if (invalid && !messages.some(function (message) {
          if (message.kind === "snapshot") return true;
          if (message.kind !== "documents") return false;
          var replacements = JSON.parse(message.data);
          return replacements.length > 0 && replacements.every(function (document) {
            return document.messages.some(function (entry) { return entry.kind === "snapshot"; });
          }) && (messages.some(function (entry) { return entry.kind === "selection"; })
            ? JSON.parse(messages.find(function (entry) { return entry.kind === "selection"; }).data)
            : Array.from(main.children).filter(function (node) { return node.dataset.presentation !== undefined; }).map(function (node) { return node.dataset.presentation; })
          ).every(function (id) {
            return replacements.some(function (document) { return document.id === id; });
          });
        }))
          throw Error("presentation requires a replacement snapshot");
        if (messages.some(function (message) { return message.kind !== "status"; })) connectionStatus("");
        messages.forEach(function (message) {
          if (message.kind === "selection") {
            var selected = JSON.parse(message.data);
            Array.from(main.children).forEach(function (node) {
              if (node.dataset.presentation !== undefined && !selected.includes(node.dataset.presentation)) {
                documentStreams.delete(node.dataset.presentation); node.remove();
              }
            });
          }
          else if (message.kind === "documents") documents(JSON.parse(message.data));
          else if (message.kind === "snapshot") snapshot(message.data);
          else if (message.kind === "changes") changes(JSON.parse(message.data));
          else if (message.kind === "streams") streams(JSON.parse(message.data));
          else if (message.kind === "stream") streamUpdate(JSON.parse(message.data));
          else if (message.kind === "status") connectionStatus(message.data);
          else throw Error("unknown presentation transaction member");
        });
        main.hidden = false;
        invalid = false;
        if (recoveryFocus) {
          var draft = composer();
          if (draft) {
            draft.focus({ preventScroll: true });
            draft.setSelectionRange(recoveryFocus[0], recoveryFocus[1]);
          }
          recoveryFocus = null;
        }
      } catch (error) {
        // A DOM mismatch invalidates this derived cache. Do not expose partial
        // content while the new SSE subscription captures a coherent snapshot.
        invalidate();
        throw error;
      }
    }
    function documents(updates) {
      var root = main, previousStreams = activeStreams, previousPrefix = documentPrefix;
      try {
        updates.forEach(function (update) {
          var region = Array.from(root.children).find(function (node) { return node.dataset.presentation === update.id; });
          if (!region) {
            region = doc.createElement("section");
            region.dataset.presentation = update.id;
            root.appendChild(region);
          }
          main = region;
          documentPrefix = update.prefix;
          if (!documentStreams.has(update.id)) documentStreams.set(update.id, new Map());
          activeStreams = documentStreams.get(update.id);
          update.messages.forEach(function (message) {
            if (message.kind === "snapshot") snapshot(message.data);
            else if (message.kind === "changes") changes(JSON.parse(message.data));
            else if (message.kind === "streams") streams(JSON.parse(message.data));
            else if (message.kind === "stream") streamUpdate(JSON.parse(message.data));
            else if (message.kind === "status") notice(documentPrefix + "presentation-status", message.data);
            else throw Error("unknown document update");
          });
        });
      } finally {
        main = root; activeStreams = previousStreams; documentPrefix = previousPrefix;
      }
    }
    if (browser.EventSource) {
      function connect() {
        if (presentationClosed) return;
        var stream = eventStream = new browser.EventSource("./events");
        stream.addEventListener("transaction", function (message) {
          try { transaction(JSON.parse(message.data)); }
          catch (_) {
            invalidate();
            stream.close();
            browser.setTimeout(connect, 250);
          }
        });
      }
      connect();
    }
    return {
      prefs: prefs,
      snapshot: snapshot,
      changes: changes,
      streams: streams,
      streamUpdate: streamUpdate,
      transaction: transaction,
    };
  }
  global.MisaClient = { memory: memory, start: start };
  if (global.document) start(global.document, global);
})(globalThis);
