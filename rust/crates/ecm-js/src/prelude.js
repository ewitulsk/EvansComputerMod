// The web API for page scripts, on the natives in __n (ecm-js/src/lib.rs).
// Node ids: the document is 0, the window is -1 (for listeners).
(function (g) {
  "use strict";
  var n = g.__n;
  var wrappers = new Map();
  var listeners = new Map(); // id -> { type: [fn, ...] }

  function wrap(id) {
    if (id === undefined || id === null || id < 0) return null;
    if (id === 0) return document;
    var w = wrappers.get(id);
    if (w) return w;
    var kind = n.kind(id);
    if (kind === 0) return null;
    w = kind === 3 ? new Text(id) : new Element(id);
    wrappers.set(id, w);
    return w;
  }
  function wrapAll(ids) {
    var out = [];
    for (var i = 0; i < ids.length; i++) out.push(wrap(ids[i]));
    return out;
  }
  function idOf(x) {
    if (x === document) return 0;
    if (x && typeof x._id === "number") return x._id;
    throw new TypeError("not a node");
  }
  function toNode(x) {
    return typeof x === "string" || typeof x === "number" ? wrap(n.createText(String(x))) : x;
  }

  // ---------------------------------------------------------------- events

  function Event(type, init) {
    this.type = type;
    this.target = null;
    this.currentTarget = null;
    this.defaultPrevented = false;
    this.bubbles = !init || init.bubbles !== false;
    this._stop = false;
    if (init) for (var k in init) if (!(k in this)) this[k] = init[k];
  }
  Event.prototype.preventDefault = function () { this.defaultPrevented = true; };
  Event.prototype.stopPropagation = function () { this._stop = true; };
  Event.prototype.stopImmediatePropagation = function () { this._stop = true; };
  g.Event = Event;

  function listen(id, type, fn) {
    if (typeof fn !== "function" && !(fn && typeof fn.handleEvent === "function")) return;
    var m = listeners.get(id);
    if (!m) { m = {}; listeners.set(id, m); }
    (m[type] = m[type] || []).indexOf(fn) < 0 && m[type].push(fn);
  }
  function unlisten(id, type, fn) {
    var m = listeners.get(id);
    if (!m || !m[type]) return;
    var i = m[type].indexOf(fn);
    if (i >= 0) m[type].splice(i, 1);
  }
  function fire(id, ev) {
    var target = id === -1 ? g : wrap(id);
    var m = listeners.get(id);
    var fns = m && m[ev.type] ? m[ev.type].slice() : [];
    // on<type> properties (onclick = ...) too.
    var prop = target && target["on" + ev.type];
    if (typeof prop === "function") fns.push(prop);
    ev.currentTarget = target;
    for (var i = 0; i < fns.length; i++) {
      try {
        if (typeof fns[i] === "function") fns[i].call(target, ev); else fns[i].handleEvent(ev);
      } catch (e) {
        if (String(e).indexOf("interrupted") >= 0) throw e;
        n.log("error", String(e && e.stack ? e + "\n" + e.stack : e));
      }
      if (ev._stop && ev.type !== "load") break;
    }
  }
  // Fire `ev` at node `id` and up through its ancestors, the document and the window.
  function dispatchAt(id, ev) {
    ev.target = id === -1 ? g : wrap(id);
    var path = [];
    if (id !== -1) {
      for (var p = id; p >= 0; p = n.parent(p)) { path.push(p); if (p === 0) break; }
    }
    path.push(-1);
    for (var i = 0; i < path.length; i++) {
      fire(path[i], ev);
      if (ev._stop || (!ev.bubbles && i === 0)) break;
    }
    return ev.defaultPrevented;
  }
  // Called by the Browser: true if prevented.
  g.__dispatch = function (id, type, key) {
    var ev = new Event(type, { bubbles: type !== "load" && type !== "DOMContentLoaded" && type !== "focus" && type !== "blur" });
    if (key) { ev.key = key; ev.code = key; }
    return dispatchAt(id, ev);
  };

  // ---------------------------------------------------------------- nodes

  function Node(id) { this._id = id; }
  Node.prototype = {
    get nodeType() { return n.kind(this._id); },
    get parentNode() { return wrap(n.parent(this._id)); },
    get parentElement() { var p = n.parent(this._id); return p > 0 ? wrap(p) : null; },
    get childNodes() { return wrapAll(n.children(this._id, false)); },
    get firstChild() { var c = n.children(this._id, false); return c.length ? wrap(c[0]) : null; },
    get lastChild() { var c = n.children(this._id, false); return c.length ? wrap(c[c.length - 1]) : null; },
    get nextSibling() { return sibling(this._id, 1, false); },
    get previousSibling() { return sibling(this._id, -1, false); },
    get textContent() { return n.text(this._id); },
    set textContent(v) { n.setText(this._id, v === null || v === undefined ? "" : String(v)); },
    get isConnected() { for (var p = this._id; p >= 0; p = n.parent(p)) if (p === 0) return true; return false; },
    appendChild: function (c) { c = toNode(c); n.insert(this._id, idOf(c), -1); return c; },
    insertBefore: function (c, ref) { c = toNode(c); n.insert(this._id, idOf(c), ref ? idOf(ref) : -1); return c; },
    removeChild: function (c) { n.detach(idOf(c)); return c; },
    replaceChild: function (nc, old) { nc = toNode(nc); n.insert(this._id, idOf(nc), idOf(old)); n.detach(idOf(old)); return old; },
    contains: function (o) { for (var p = idOf(o); p >= 0; p = n.parent(p)) if (p === this._id) return true; return false; },
    addEventListener: function (t, fn) { listen(this._id, t, fn); },
    removeEventListener: function (t, fn) { unlisten(this._id, t, fn); },
    dispatchEvent: function (ev) { return !dispatchAt(this._id, ev); },
  };
  function sibling(id, dir, elements) {
    var p = n.parent(id);
    if (p < 0) return null;
    var c = n.children(p, elements);
    var i = c.indexOf(id);
    if (i < 0) return null;
    var j = i + dir;
    return j >= 0 && j < c.length ? wrap(c[j]) : null;
  }
  g.Node = Node;

  function Text(id) { Node.call(this, id); }
  Text.prototype = Object.create(Node.prototype);
  Object.defineProperty(Text.prototype, "data", { get: function () { return n.text(this._id); }, set: function (v) { n.setText(this._id, String(v)); } });
  Object.defineProperty(Text.prototype, "nodeName", { get: function () { return "#text"; } });
  Text.prototype.remove = function () { n.detach(this._id); };
  g.Text = Text;

  function ClassList(id) { this._id = id; }
  ClassList.prototype = {
    _get: function () { var c = n.attr(this._id, "class"); return c ? c.split(/\s+/).filter(Boolean) : []; },
    _set: function (l) { n.setAttr(this._id, "class", l.join(" ")); },
    contains: function (c) { return this._get().indexOf(c) >= 0; },
    add: function () { var l = this._get(); for (var i = 0; i < arguments.length; i++) if (l.indexOf(arguments[i]) < 0) l.push(arguments[i]); this._set(l); },
    remove: function () { var a = Array.prototype.slice.call(arguments); this._set(this._get().filter(function (c) { return a.indexOf(c) < 0; })); },
    toggle: function (c, force) {
      var has = this.contains(c);
      var on = force === undefined ? !has : !!force;
      if (on && !has) this.add(c);
      if (!on && has) this.remove(c);
      return on;
    },
    get length() { return this._get().length; },
    item: function (i) { return this._get()[i] || null; },
    toString: function () { return this._get().join(" "); },
  };

  function styleProxy(id) {
    return new Proxy({}, {
      get: function (_, prop) {
        if (prop === "setProperty") return function (k, v) { n.setStyle(id, k, String(v)); };
        if (prop === "getPropertyValue") return function (k) { return n.style(id, k); };
        if (prop === "removeProperty") return function (k) { var v = n.style(id, k); n.setStyle(id, k, ""); return v; };
        if (prop === "cssText") return n.attr(id, "style") || "";
        return typeof prop === "string" ? n.style(id, prop) : undefined;
      },
      set: function (_, prop, v) {
        if (prop === "cssText") n.setAttr(id, "style", String(v));
        else n.setStyle(id, prop, v === null || v === undefined ? "" : String(v));
        return true;
      },
    });
  }

  function Element(id) { Node.call(this, id); }
  Element.prototype = Object.create(Node.prototype);
  var EP = {
    get tagName() { return n.tag(this._id).toUpperCase(); },
    get nodeName() { return n.tag(this._id).toUpperCase(); },
    get localName() { return n.tag(this._id); },
    get id() { return n.attr(this._id, "id") || ""; },
    set id(v) { n.setAttr(this._id, "id", String(v)); },
    get className() { return n.attr(this._id, "class") || ""; },
    set className(v) { n.setAttr(this._id, "class", String(v)); },
    get classList() { return new ClassList(this._id); },
    get style() { return styleProxy(this._id); },
    get innerHTML() { return n.html(this._id, false); },
    set innerHTML(v) { n.setHtml(this._id, String(v)); },
    get outerHTML() { return n.html(this._id, true); },
    get innerText() { return n.text(this._id); },
    set innerText(v) { n.setText(this._id, String(v)); },
    get value() { return n.value(this._id); },
    set value(v) { n.setValue(this._id, String(v)); },
    get checked() { return n.attr(this._id, "checked") !== null && n.attr(this._id, "checked") !== undefined; },
    set checked(v) { if (v) n.setAttr(this._id, "checked", ""); else n.removeAttr(this._id, "checked"); },
    get disabled() { return n.attr(this._id, "disabled") != null; },
    set disabled(v) { if (v) n.setAttr(this._id, "disabled", ""); else n.removeAttr(this._id, "disabled"); },
    get hidden() { return n.attr(this._id, "hidden") != null; },
    set hidden(v) { if (v) n.setAttr(this._id, "hidden", ""); else n.removeAttr(this._id, "hidden"); },
    get href() { var h = n.attr(this._id, "href"); return h == null ? "" : (n.resolve(h) || h); },
    set href(v) { n.setAttr(this._id, "href", String(v)); },
    get src() { var h = n.attr(this._id, "src"); return h == null ? "" : (n.resolve(h) || h); },
    set src(v) { n.setAttr(this._id, "src", String(v)); },
    get name() { return n.attr(this._id, "name") || ""; },
    get type() { return n.attr(this._id, "type") || ""; },
    get children() { return wrapAll(n.children(this._id, true)); },
    get childElementCount() { return n.children(this._id, true).length; },
    get firstElementChild() { var c = n.children(this._id, true); return c.length ? wrap(c[0]) : null; },
    get lastElementChild() { var c = n.children(this._id, true); return c.length ? wrap(c[c.length - 1]) : null; },
    get nextElementSibling() { return sibling(this._id, 1, true); },
    get previousElementSibling() { return sibling(this._id, -1, true); },
    get dataset() {
      var id = this._id;
      return new Proxy({}, {
        get: function (_, k) { return typeof k === "string" ? n.attr(id, "data-" + k.replace(/[A-Z]/g, function (c) { return "-" + c.toLowerCase(); })) : undefined; },
        set: function (_, k, v) { n.setAttr(id, "data-" + String(k).replace(/[A-Z]/g, function (c) { return "-" + c.toLowerCase(); }), String(v)); return true; },
      });
    },
    getAttribute: function (k) { var v = n.attr(this._id, k); return v === undefined ? null : v; },
    setAttribute: function (k, v) { n.setAttr(this._id, k, String(v)); },
    removeAttribute: function (k) { n.removeAttr(this._id, k); },
    hasAttribute: function (k) { return n.attr(this._id, k) != null; },
    getAttributeNames: function () { return n.attrNames(this._id); },
    toggleAttribute: function (k, force) {
      var has = this.hasAttribute(k), on = force === undefined ? !has : !!force;
      if (on) n.setAttr(this._id, k, ""); else n.removeAttr(this._id, k);
      return on;
    },
    querySelector: function (s) { return wrap(n.query(this._id, s, false)[0]); },
    querySelectorAll: function (s) { return wrapAll(n.query(this._id, s, true)); },
    getElementsByTagName: function (t) { return wrapAll(n.query(this._id, t, true)); },
    getElementsByClassName: function (c) { return wrapAll(n.query(this._id, "." + c.trim().split(/\s+/).join("."), true)); },
    matches: function (s) { return n.matches(this._id, s); },
    closest: function (s) { for (var p = this._id; p > 0; p = n.parent(p)) if (n.kind(p) === 1 && n.matches(p, s)) return wrap(p); return null; },
    remove: function () { n.detach(this._id); },
    append: function () { for (var i = 0; i < arguments.length; i++) this.appendChild(toNode(arguments[i])); },
    prepend: function () { var f = this.firstChild; for (var i = 0; i < arguments.length; i++) this.insertBefore(toNode(arguments[i]), f); },
    before: function () { var p = this.parentNode; for (var i = 0; i < arguments.length; i++) p.insertBefore(toNode(arguments[i]), this); },
    after: function () { var p = this.parentNode, nx = this.nextSibling; for (var i = 0; i < arguments.length; i++) p.insertBefore(toNode(arguments[i]), nx); },
    replaceWith: function (x) { this.parentNode.replaceChild(toNode(x), this); },
    click: function () { dispatchAt(this._id, new Event("click")); },
    focus: function () {},
    blur: function () {},
    submit: function () { dispatchAt(this._id, new Event("submit")); },
  };
  for (var k in EP) Object.defineProperty(Element.prototype, k, Object.getOwnPropertyDescriptor(EP, k));
  g.Element = Element;
  g.HTMLElement = Element;

  // ---------------------------------------------------------------- document

  var document = Object.create(Node.prototype);
  document._id = 0;
  var DP = {
    get documentElement() { return wrap(n.query(0, "html", false)[0]) || null; },
    get body() { return wrap(n.query(0, "body", false)[0]) || null; },
    get head() { return wrap(n.query(0, "head", false)[0]) || null; },
    get title() { return n.title(); },
    set title(v) { n.setTitle(String(v)); },
    get readyState() { return "complete"; },
    get location() { return g.location; },
    get nodeType() { return 9; },
    getElementById: function (id) { var i = n.byId(String(id)); return i < 0 ? null : wrap(i); },
    querySelector: function (s) { return wrap(n.query(0, s, false)[0]); },
    querySelectorAll: function (s) { return wrapAll(n.query(0, s, true)); },
    getElementsByTagName: function (t) { return wrapAll(n.query(0, t, true)); },
    getElementsByClassName: function (c) { return wrapAll(n.query(0, "." + c.trim().split(/\s+/).join("."), true)); },
    getElementsByName: function (nm) { return wrapAll(n.query(0, "[name=\"" + nm + "\"]", true)); },
    createElement: function (t) { return wrap(n.create(String(t))); },
    createTextNode: function (t) { return wrap(n.createText(String(t))); },
    createDocumentFragment: function () { return wrap(n.create("div")); },
  };
  for (var k2 in DP) Object.defineProperty(document, k2, Object.getOwnPropertyDescriptor(DP, k2));
  g.document = document;

  // ---------------------------------------------------------------- timers

  var timers = [], nextTimer = 1, frames = [];
  function now() { return g.performance.now(); }
  var t0 = Date.now();
  g.performance = { now: function () { return Date.now() - t0; } };
  g.setTimeout = function (fn, ms) {
    var args = Array.prototype.slice.call(arguments, 2);
    var id = nextTimer++;
    timers.push({ id: id, due: now() + Math.max(0, +ms || 0), fn: fn, args: args, every: 0 });
    return id;
  };
  g.setInterval = function (fn, ms) {
    var args = Array.prototype.slice.call(arguments, 2);
    var id = nextTimer++, every = Math.max(10, +ms || 0);
    timers.push({ id: id, due: now() + every, fn: fn, args: args, every: every });
    return id;
  };
  g.clearTimeout = g.clearInterval = function (id) { timers = timers.filter(function (t) { return t.id !== id; }); };
  g.requestAnimationFrame = function (fn) { var id = nextTimer++; frames.push({ id: id, fn: fn }); return id; };
  g.cancelAnimationFrame = function (id) { frames = frames.filter(function (f) { return f.id !== id; }); };
  g.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
  // Called by the Browser each turn: run what is due; the next due time (ms) or -1.
  g.__tick = function () {
    var t = now();
    var due = timers.filter(function (x) { return x.due <= t; }).sort(function (a, b) { return a.due - b.due; });
    for (var i = 0; i < due.length; i++) {
      var x = due[i];
      if (timers.indexOf(x) < 0) continue;
      if (x.every) x.due = t + x.every; else timers.splice(timers.indexOf(x), 1);
      try {
        if (typeof x.fn === "function") x.fn.apply(g, x.args); else (0, eval)(String(x.fn));
      } catch (e) {
        if (String(e).indexOf("interrupted") >= 0) throw e;
        n.log("error", String(e));
      }
    }
    var fs = frames; frames = [];
    for (var j = 0; j < fs.length; j++) {
      try { fs[j].fn(t); } catch (e) { if (String(e).indexOf("interrupted") >= 0) throw e; n.log("error", String(e)); }
    }
    if (frames.length) return t;
    var next = -1;
    for (var k = 0; k < timers.length; k++) if (next < 0 || timers[k].due < next) next = timers[k].due;
    return next;
  };
  g.__pending = function () { return timers.length > 0 || frames.length > 0; };

  // ---------------------------------------------------------------- console, dialogs

  function fmt(args) {
    var out = [];
    for (var i = 0; i < args.length; i++) {
      var a = args[i];
      if (typeof a === "string") out.push(a);
      else if (a instanceof Error) out.push(String(a));
      else if (a instanceof Node) out.push("<" + (a.tagName ? a.tagName.toLowerCase() : "#text") + ">");
      else { try { out.push(JSON.stringify(a)); } catch (e) { out.push(String(a)); } }
    }
    return out.join(" ");
  }
  g.console = {};
  ["log", "info", "warn", "error", "debug"].forEach(function (l) { g.console[l] = function () { n.log(l, fmt(arguments)); }; });
  g.alert = function (m) { n.modal(m === undefined ? "" : String(m), false); };
  g.confirm = function (m) { return n.modal(m === undefined ? "" : String(m), true); };
  g.prompt = function () { return null; };

  // ---------------------------------------------------------------- fetch

  function Headers(ct) { this._ct = ct; }
  Headers.prototype.get = function (k) { return String(k).toLowerCase() === "content-type" ? this._ct : null; };
  function Response(status, body, ct, url) {
    this.status = status;
    this.ok = status >= 200 && status < 300;
    this.statusText = "";
    this.url = url;
    this.headers = new Headers(ct);
    this._body = body;
  }
  Response.prototype.text = function () { return Promise.resolve(this._body); };
  Response.prototype.json = function () {
    var b = this._body;
    return new Promise(function (ok, fail) { try { ok(JSON.parse(b)); } catch (e) { fail(e); } });
  };
  g.fetch = function (url, opts) {
    opts = opts || {};
    var method = (opts.method || "GET").toUpperCase();
    var body = opts.body === undefined || opts.body === null ? undefined : String(opts.body);
    var r = n.fetch(method, String(url), body);
    if (r[0] !== "ok") return Promise.reject(new TypeError("fetch failed: " + r[1]));
    return Promise.resolve(new Response(+r[1], r[2], r[3], r[4]));
  };

  // ---------------------------------------------------------------- storage, location, window

  g.localStorage = {
    getItem: function (k) { var v = n.storage("get", String(k), ""); return v === undefined ? null : v; },
    setItem: function (k, v) { n.storage("set", String(k), String(v)); },
    removeItem: function (k) { n.storage("remove", String(k), ""); },
    clear: function () { n.storage("clear", "", ""); },
    key: function (i) { var v = n.storage("key", String(i), ""); return v === undefined ? null : v; },
    get length() { return +n.storage("length", "", ""); },
  };
  g.sessionStorage = g.localStorage;
  g.location = {
    get href() { return n.url(); },
    set href(v) { n.navigate("go", String(v)); },
    assign: function (v) { n.navigate("go", String(v)); },
    replace: function (v) { n.navigate("go", String(v)); },
    reload: function () { n.navigate("reload", ""); },
    toString: function () { return n.url(); },
    get pathname() { var u = n.url(); var m = u.match(/^[a-z]+:\/\/[^\/]*(\/[^?#]*)/); return m ? m[1] : "/"; },
    get search() { var u = n.url().split("#")[0]; var i = u.indexOf("?"); return i < 0 ? "" : u.slice(i); },
    get hash() { var u = n.url(); var i = u.indexOf("#"); return i < 0 ? "" : u.slice(i); },
    get host() { var m = n.url().match(/^[a-z]+:\/\/([^\/?#]*)/); return m ? m[1] : ""; },
    get hostname() { return this.host.split(":")[0]; },
    get protocol() { return n.url().split(":")[0] + ":"; },
  };
  g.history = {
    back: function () { n.navigate("back", ""); },
    forward: function () { n.navigate("forward", ""); },
    go: function (d) { n.navigate(d < 0 ? "back" : d > 0 ? "forward" : "reload", ""); },
  };
  g.navigator = { userAgent: "ECM-Browser/1.0", language: "en" };
  Object.defineProperty(g, "innerWidth", { get: function () { return n.viewport()[0]; } });
  Object.defineProperty(g, "innerHeight", { get: function () { return n.viewport()[1]; } });
  g.addEventListener = function (t, fn) { listen(-1, t, fn); };
  g.removeEventListener = function (t, fn) { unlisten(-1, t, fn); };
  g.window = g;
  g.self = g;
})(globalThis);
