//! web.py on a real RustPython interpreter, loaded as the bootstrap loads it:
//! requests through `web.handle`, CGI input through `web.parse_cgi`.

use super::*;
use rustpython_vm::{compiler::Mode, Interpreter, Settings};

const SETUP: &str = "import sys, _web\nM = type(sys)('web')\nexec(compile(_web._source, '<web>', 'exec'), M.__dict__)\nsys.modules['web'] = M\nimport web\n";

/// Run `code` and return the value of `OUT` (a str).
fn run(code: &str) -> String {
    let interp = Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_web".to_owned(), Box::new(web_native::make_module));
    });
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let src = format!("{SETUP}{code}");
        let obj = vm.compile(&src, Mode::Exec, "<test>".to_owned()).unwrap_or_else(|e| panic!("{e:?} in:\n{src}"));
        if let Err(e) = vm.run_code_obj(obj, scope.clone()) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
        let out = scope.globals.get_item("OUT", vm).expect("OUT");
        out.str(vm).unwrap().as_str().to_string()
    })
}

const HANDLER: &str = r#"
def h(req):
    if req.path == "/boom":
        raise ValueError("broken")
    if req.path == "/none":
        return None
    if req.method == "POST":
        if not req.form.get("name"):
            return web.Response("<p>Name required</p>", 400)
        r = web.redirect("/")
        r.set_cookie("who", req.form["name"])
        return r
    return "<p>hi " + web.escape(req.query.get("n", "")) + " " + req.cookies.get("who", "-") + "</p>"
"#;

#[test]
fn requests_through_a_handler() {
    let code = r#"
OUT = repr([
    web.handle(b"GET /?n=a%26b+c HTTP/1.0\r\nCookie: who=Pat\r\n\r\n", h),
    web.handle(b"POST /sign HTTP/1.0\r\nContent-Length: 8\r\n\r\nname=Bo", h),
    web.handle(b"POST /sign HTTP/1.0\r\n\r\nname=", h),
    web.handle(b"GET /boom HTTP/1.0\r\n\r\n", h)[:36],
    web.handle(b"GET /none HTTP/1.0\r\n\r\n", h)[:24],
    web.handle(b"junk", h)[:24],
])
"#;
    let out = run(&format!("{HANDLER}{code}"));
    assert!(out.contains("<p>hi a&amp;b c Pat</p>"), "{out}");
    assert!(out.contains("HTTP/1.0 303 See Other") && out.contains("Location: /") && out.contains("Set-Cookie: who=Bo; Path=/"), "{out}");
    assert!(out.contains("HTTP/1.0 400 Bad Request") && out.contains("Name required"), "{out}");
    assert!(out.contains("HTTP/1.0 500 Internal Server"), "{out}");
    assert!(out.contains("HTTP/1.0 404 Not Found"), "{out}");
    assert!(out.contains("Content-Length: 23"), "{out}");
}

#[test]
fn cgi_requests_and_output() {
    let code = r#"
r = web.parse_cgi("REQUEST_METHOD=POST\nPATH_INFO=/cgi/g.py\nQUERY_STRING=x=1\nCONTENT_LENGTH=11\nHTTP_COOKIE=a=b\n\nname=Al+Bo")
OUT = repr([r.method, r.path, r.query, r.form, r.cookies, web.cgi_output(web._answer(h, r))])
"#;
    let out = run(&format!("{HANDLER}{code}"));
    assert!(out.contains("'POST', '/cgi/g.py', {'x': '1'}, {'name': 'Al Bo'}, {'a': 'b'}"), "{out}");
    assert!(out.contains(r"Status: 303 See Other\nContent-Type: text/html\nLocation: /\nSet-Cookie: who=Al Bo; Path=/\n\n"), "{out}");
    assert_eq!(run("OUT = web.parse_query('a=1&b=two+words&c=%C3%A9&a=2')"), "{'a': '2', 'b': 'two words', 'c': 'é'}");
}
