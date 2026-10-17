-- web: small web servers and CGI scripts for the Browser (docs/gui/BROWSER.md).
-- The same names as Python's web (web.py).
--
--   local web = require("web")
--
--   local function hello(req)
--     local name = req.query.name or "world"
--     return "<h1>Hello, " .. web.escape(name) .. "</h1>"
--   end
--
--   web.serve(8080, hello)    -- a server of its own on port 8080 (Ctrl+T stops it)
--   web.cgi(hello)            -- or one request handed over by: httpd 80 /srv/www --cgi /cgi
--
-- A handler gets a request table and returns a string (an HTML page), a
-- response (web.Response, web.redirect), or nil (404 Not Found). An error
-- becomes a 500 page with its message.
--
-- Request: method, path, query, form (a POSTed form), headers (lower-case
-- names), cookies, body, remote. web.Response(body, status, headers,
-- content_type) has resp:set_cookie(name, value).

local web = {}

web.REASONS = {
  [200] = "OK", [201] = "Created", [204] = "No Content", [301] = "Moved Permanently", [302] = "Found",
  [303] = "See Other", [304] = "Not Modified", [400] = "Bad Request", [401] = "Unauthorized", [403] = "Forbidden",
  [404] = "Not Found", [405] = "Method Not Allowed", [413] = "Payload Too Large", [500] = "Internal Server Error",
}

web.MAX_BODY = 65536

local function unquote(s)
  s = s:gsub("+", " ")
  return (s:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end))
end

--- "a=1&b=two+words" -> {a = "1", b = "two words"} (a name given twice: the last value).
function web.parse_query(s)
  local out = {}
  for part in string.gmatch(s or "", "[^&]+") do
    local k, v = part:match("^([^=]*)=(.*)$")
    if not k then k, v = part, "" end
    out[unquote(k)] = unquote(v)
  end
  return out
end

--- Text made safe to put in a page.
function web.escape(s)
  s = tostring(s)
  return (s:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;"):gsub('"', "&quot;"):gsub("'", "&#39;"))
end

local function trim(s)
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

--- A request: what a handler gets.
function web.Request(method, target, headers, body, remote)
  method = string.upper(method or "GET")
  target = target or "/"
  local path, query = target:match("^([^?]*)%??(.*)$")
  local req = {
    method = method, path = (path ~= "" and path) or "/", query = web.parse_query(query),
    headers = {}, body = body or "", remote = remote or "", cookies = {}, form = {},
  }
  for k, v in pairs(headers or {}) do req.headers[string.lower(k)] = v end
  for part in string.gmatch(req.headers["cookie"] or "", "[^;]+") do
    local k, v = part:match("^%s*([^=]+)=(.*)$")
    if k then req.cookies[trim(k)] = trim(v) end
  end
  local ctype = req.headers["content-type"] or ""
  if method == "POST" and (ctype == "" or ctype:find("urlencoded", 1, true)) then
    req.form = web.parse_query(req.body)
  end
  return req
end

local Response = {}
Response.__index = Response

--- A response: body, status (200), headers (a table of name = value), content type (text/html).
function web.Response(body, status, headers, content_type)
  local r = setmetatable({ body = body or "", status = status or 200, headers = {}, content_type = content_type or "text/html" }, Response)
  for k, v in pairs(headers or {}) do table.insert(r.headers, { k, v }) end
  return r
end

function Response:set_cookie(name, value)
  table.insert(self.headers, { "Set-Cookie", name .. "=" .. value .. "; Path=/" })
  return self
end

--- Send the browser to another address (after a POST: 303, so it GETs it).
function web.redirect(location, status)
  return web.Response("", status or 303, { Location = location })
end

local function answer(handler, req)
  local ok, r = pcall(handler, req)
  if not ok then
    return web.Response("<h1>500 Internal Server Error</h1><pre>" .. web.escape(tostring(r)) .. "</pre>", 500)
  end
  if r == nil then
    return web.Response("<h1>404 Not Found</h1><p>" .. web.escape(req.path) .. "</p>", 404)
  end
  if type(r) == "table" and getmetatable(r) == Response then return r end
  if type(r) == "table" then
    return web.Response(r.body, r.status, r.headers, r.content_type)
  end
  return web.Response(tostring(r))
end
web._answer = answer

local function http_bytes(resp)
  local body = resp.body
  local ct = resp.content_type
  if ct:sub(1, 5) == "text/" then ct = ct .. "; charset=utf-8" end
  local lines = {
    string.format("HTTP/1.0 %d %s", resp.status, web.REASONS[resp.status] or "OK"),
    "Content-Type: " .. ct, "Content-Length: " .. #body, "Connection: close",
  }
  for _, h in ipairs(resp.headers) do table.insert(lines, h[1] .. ": " .. h[2]) end
  return table.concat(lines, "\r\n") .. "\r\n\r\n" .. body
end

local function parse_http(raw, remote)
  local s, e = raw:find("\r\n\r\n", 1, true)
  if not s then return nil end
  local head = raw:sub(1, s - 1)
  local first = head:match("^[^\r\n]*")
  local method, target = first:match("^(%S+)%s+(%S+)")
  if not method then return nil end
  local headers = {}
  for line in head:gmatch("\r\n([^\r\n]*)") do
    local k, v = line:match("^([^:]+):(.*)$")
    if k then headers[trim(k)] = trim(v) end
  end
  return web.Request(method, target, headers, raw:sub(e + 1), remote)
end

--- One HTTP request (a string) through `handler`: the response as a string.
function web.handle(raw, handler, remote)
  local req = parse_http(raw, remote)
  if not req then return http_bytes(web.Response("Bad Request", 400, nil, "text/plain")) end
  return http_bytes(answer(handler, req))
end

local function read_request(net, conn)
  local data = ""
  while not data:find("\r\n\r\n", 1, true) and #data < web.MAX_BODY do
    local chunk = net.tcp_recv(conn, 4096)
    if chunk == "" then return data end
    data = data .. chunk
  end
  local s, e = data:find("\r\n\r\n", 1, true)
  local length = tonumber(data:sub(1, s):lower():match("\r\ncontent%-length:%s*(%d+)")) or 0
  while #data - e < math.min(length, web.MAX_BODY) do
    local chunk = net.tcp_recv(conn, 4096)
    if chunk == "" then break end
    data = data .. chunk
  end
  return data
end

--- Answer HTTP requests on `port` with `handler`, one at a time, until stopped (Ctrl+T).
function web.serve(port, handler, log)
  local net = require("net")
  if log == nil then log = true end
  local listener = net.tcp_listen(port)
  if log then print(string.format("web: serving on port %d. Ctrl+T to stop.", port)) end
  while true do
    local ok, conn = pcall(net.tcp_accept, listener)
    if ok then
      local served, err = pcall(function()
        local raw = read_request(net, conn)
        if raw ~= "" then
          local req = parse_http(raw)
          local resp = req and answer(handler, req) or web.Response("Bad Request", 400, nil, "text/plain")
          if log then print(string.format("%s %s %d", req and req.method or "?", req and req.path or "?", resp.status)) end
          net.tcp_send(conn, http_bytes(resp))
        end
      end)
      if not served and log then print("web: " .. tostring(err)) end
      net.tcp_close(conn)
    end
  end
end

--- What httpd hands a CGI script (NAME=value lines, a blank line, the body) as a request.
function web.parse_cgi(text)
  local head, body = text:match("^(.-)\n\n(.*)$")
  if not head then head, body = text, "" end
  local env = {}
  for line in (head .. "\n"):gmatch("([^\n]*)\n") do
    local k, v = line:match("^([^=]+)=(.*)$")
    if k then env[trim(k)] = (v:gsub("\r$", "")) end
  end
  local headers = {}
  for k, v in pairs(env) do
    if k:sub(1, 5) == "HTTP_" then headers[(k:sub(6):gsub("_", "-"))] = v end
  end
  if env.CONTENT_TYPE and env.CONTENT_TYPE ~= "" then headers["content-type"] = env.CONTENT_TYPE end
  local target = (env.PATH_INFO or "/")
  if env.QUERY_STRING and env.QUERY_STRING ~= "" then target = target .. "?" .. env.QUERY_STRING end
  local length = tonumber(env.CONTENT_LENGTH or "0") or 0
  return web.Request(env.REQUEST_METHOD or "GET", target, headers, body:sub(1, length), env.REMOTE_ADDR or "")
end

--- A response as a CGI script prints it: Status and headers, a blank line, the body.
function web.cgi_output(resp)
  local lines = { string.format("Status: %d %s", resp.status, web.REASONS[resp.status] or "OK"), "Content-Type: " .. resp.content_type }
  for _, h in ipairs(resp.headers) do table.insert(lines, h[1] .. ": " .. h[2]) end
  return table.concat(lines, "\n") .. "\n\n" .. resp.body
end

--- Answer the one request httpd hands this script (on stdin), on stdout.
function web.cgi(handler)
  local head = ""
  while true do
    local line = io.read("*l")
    if not line or line:gsub("\r", "") == "" then break end
    head = head .. line .. "\n"
  end
  local length = math.min(tonumber(head:match("CONTENT_LENGTH=(%d+)") or "0") or 0, web.MAX_BODY)
  local body = ""
  while #body < length do
    local chunk = io.read(length - #body)
    if not chunk then break end
    body = body .. chunk
  end
  local req = web.parse_cgi(head .. "\n" .. body)
  io.write(web.cgi_output(answer(handler, req)))
  io.flush()
end

return web
