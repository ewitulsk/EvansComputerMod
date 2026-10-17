"""web: small web servers and CGI scripts for the Browser (docs/gui/BROWSER.md).

The same module exists in Lua (`local web = require("web")`) with the same names.

    import web

    def hello(req):
        name = req.query.get("name", "world")
        return "<h1>Hello, " + web.escape(name) + "</h1>"

    web.serve(8080, hello)    # a server of its own on port 8080 (Ctrl+T stops it)
    web.cgi(hello)            # or one request handed over by: httpd 80 /srv/www --cgi /cgi

A handler gets a Request and returns a str (an HTML page), a Response, or
None (404 Not Found). An exception becomes a 500 page with its message.

Request: method, path, query (dict), form (dict: a POSTed form), headers
(dict, lower-case names), cookies (dict), body (str), remote (the client's
address).
Response(body="", status=200, headers=None, content_type="text/html"):
.set_cookie(name, value) adds a Set-Cookie header.
"""

import sys

REASONS = {200: "OK", 201: "Created", 204: "No Content", 301: "Moved Permanently", 302: "Found", 303: "See Other",
           304: "Not Modified", 400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found",
           405: "Method Not Allowed", 413: "Payload Too Large", 500: "Internal Server Error"}

MAX_BODY = 65536


def _hex(c):
    return "0123456789abcdef".find(c.lower())


def _unquote(s):
    """Form decoding: '+' is a space, %XX a byte (UTF-8)."""
    s = s.replace("+", " ")
    out = bytearray()
    i = 0
    raw = s.encode("utf-8")
    while i < len(raw):
        b = raw[i]
        if b == 37 and i + 2 < len(raw):
            h = _hex(chr(raw[i + 1]))
            l = _hex(chr(raw[i + 2]))
            if h >= 0 and l >= 0:
                out.append(h * 16 + l)
                i += 3
                continue
        out.append(b)
        i += 1
    return out.decode("utf-8", "replace")


def parse_query(s):
    """'a=1&b=two+words' -> {'a': '1', 'b': 'two words'} (a name given twice: the last value)."""
    out = {}
    for part in (s or "").split("&"):
        if not part:
            continue
        if "=" in part:
            k, v = part.split("=", 1)
        else:
            k, v = part, ""
        out[_unquote(k)] = _unquote(v)
    return out


def escape(s):
    """Text made safe to put in a page."""
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;").replace("'", "&#39;")


class Request:
    def __init__(self, method="GET", target="/", headers=None, body="", remote=""):
        self.method = method.upper()
        path, _, query = target.partition("?")
        self.path = path or "/"
        self.query = parse_query(query)
        self.headers = dict((k.lower(), v) for k, v in (headers or {}).items())
        self.body = body
        self.remote = remote
        self.cookies = {}
        for part in self.headers.get("cookie", "").split(";"):
            if "=" in part:
                k, v = part.split("=", 1)
                self.cookies[k.strip()] = v.strip()
        ctype = self.headers.get("content-type", "")
        self.form = parse_query(body) if self.method == "POST" and (ctype == "" or "urlencoded" in ctype) else {}

    def __repr__(self):
        return "<Request %s %s>" % (self.method, self.path)


class Response:
    def __init__(self, body="", status=200, headers=None, content_type="text/html"):
        self.body = body
        self.status = status
        self.headers = list((headers or {}).items()) if isinstance(headers, dict) else list(headers or [])
        self.content_type = content_type

    def set_cookie(self, name, value):
        self.headers.append(("Set-Cookie", "%s=%s; Path=/" % (name, value)))
        return self

    def __repr__(self):
        return "<Response %d>" % self.status


def redirect(location, status=303):
    """Send the browser to another address (after a POST: 303, so it GETs it)."""
    return Response("", status, {"Location": location})


def _answer(handler, req):
    try:
        r = handler(req)
    except Exception as e:
        return Response("<h1>500 Internal Server Error</h1><pre>" + escape(repr(e)) + "</pre>", 500)
    if r is None:
        return Response("<h1>404 Not Found</h1><p>" + escape(req.path) + "</p>", 404)
    if isinstance(r, Response):
        return r
    if isinstance(r, bytes):
        return Response(r.decode("utf-8", "replace"))
    return Response(str(r))


def _http_bytes(resp):
    body = resp.body.encode("utf-8") if isinstance(resp.body, str) else bytes(resp.body)
    lines = ["HTTP/1.0 %d %s" % (resp.status, REASONS.get(resp.status, "OK")),
             "Content-Type: " + resp.content_type + ("; charset=utf-8" if resp.content_type.startswith("text/") else ""),
             "Content-Length: %d" % len(body), "Connection: close"]
    for k, v in resp.headers:
        lines.append("%s: %s" % (k, v))
    return ("\r\n".join(lines) + "\r\n\r\n").encode("utf-8") + body


def _parse_http(raw, remote=""):
    """Raw request bytes -> Request (None if they aren't HTTP)."""
    end = raw.find(b"\r\n\r\n")
    if end < 0:
        return None
    head = raw[:end].decode("utf-8", "replace").split("\r\n")
    parts = head[0].split(" ")
    if len(parts) < 2:
        return None
    headers = {}
    for line in head[1:]:
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip()] = v.strip()
    return Request(parts[0], parts[1], headers, raw[end + 4:].decode("utf-8", "replace"), remote)


def handle(raw, handler, remote=""):
    """One HTTP request (bytes) through `handler`: the response's bytes."""
    req = _parse_http(raw, remote)
    if req is None:
        return _http_bytes(Response("Bad Request", 400, content_type="text/plain"))
    return _http_bytes(_answer(handler, req))


def _read_request(conn, net):
    data = b""
    while b"\r\n\r\n" not in data and len(data) < MAX_BODY:
        chunk = net.tcp_recv(conn, 4096)
        if not chunk:
            return data
        data += chunk
    end = data.find(b"\r\n\r\n")
    length = 0
    for line in data[:end].decode("utf-8", "replace").split("\r\n")[1:]:
        if line.lower().startswith("content-length:"):
            try:
                length = int(line.split(":", 1)[1].strip())
            except ValueError:
                length = 0
    while len(data) - end - 4 < min(length, MAX_BODY):
        chunk = net.tcp_recv(conn, 4096)
        if not chunk:
            break
        data += chunk
    return data


def serve(port, handler, log=True):
    """Answer HTTP requests on `port` with `handler`, one at a time, until stopped (Ctrl+T)."""
    import net
    listener = net.tcp_listen(port)
    if log:
        print("web: serving on port %d. Ctrl+T to stop." % port)
    while True:
        try:
            conn = net.tcp_accept(listener)
        except RuntimeError:
            continue
        try:
            raw = _read_request(conn, net)
            if raw:
                req = _parse_http(raw)
                resp = _answer(handler, req) if req else Response("Bad Request", 400, content_type="text/plain")
                if log:
                    print("%s %s %d" % (req.method if req else "?", req.path if req else "?", resp.status))
                net.tcp_send(conn, _http_bytes(resp))
        except RuntimeError as e:
            if log:
                print("web: " + str(e))
        net.tcp_close(conn)


def parse_cgi(text):
    """What httpd hands a CGI script: NAME=value lines, a blank line, the body -> Request."""
    head, _, body = text.partition("\n\n")
    env = {}
    for line in head.split("\n"):
        if "=" in line:
            k, v = line.split("=", 1)
            env[k.strip()] = v.rstrip("\r")
    headers = {}
    for k, v in env.items():
        if k.startswith("HTTP_"):
            headers[k[5:].replace("_", "-")] = v
    if env.get("CONTENT_TYPE"):
        headers["content-type"] = env["CONTENT_TYPE"]
    target = env.get("PATH_INFO", "/") + ("?" + env["QUERY_STRING"] if env.get("QUERY_STRING") else "")
    try:
        length = int(env.get("CONTENT_LENGTH", "0") or "0")
    except ValueError:
        length = 0
    return Request(env.get("REQUEST_METHOD", "GET"), target, headers, body[:length], env.get("REMOTE_ADDR", ""))


def cgi_output(resp):
    """A response as a CGI script prints it: Status and headers, a blank line, the body."""
    lines = ["Status: %d %s" % (resp.status, REASONS.get(resp.status, "OK")), "Content-Type: " + resp.content_type]
    for k, v in resp.headers:
        lines.append("%s: %s" % (k, v))
    body = resp.body if isinstance(resp.body, str) else resp.body.decode("utf-8", "replace")
    return "\n".join(lines) + "\n\n" + body


def cgi(handler):
    """Answer the one request httpd hands this script (on stdin), on stdout."""
    head = ""
    while True:
        line = sys.stdin.readline()
        if not line or line.strip() == "":
            break
        head += line
    length = 0
    for l in head.splitlines():
        if l.startswith("CONTENT_LENGTH="):
            try:
                length = int(l.split("=", 1)[1].strip() or "0")
            except ValueError:
                length = 0
    length = min(length, MAX_BODY)
    body = ""
    while len(body) < length:
        chunk = sys.stdin.read(length - len(body))
        if not chunk:
            break
        body += chunk
    req = parse_cgi(head + chr(10) + body)
    sys.stdout.write(cgi_output(_answer(handler, req)))
    sys.stdout.flush()
