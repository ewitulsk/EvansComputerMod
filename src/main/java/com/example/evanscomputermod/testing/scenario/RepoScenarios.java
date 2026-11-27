package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * TLS and package repository scenarios (docs/TLS.md, docs/REPO.md), spawnable
 * with {@code /ecm scenario spawn <name>} and run by the {@code ecm_repo}
 * GameTests. Built and operated by a player: computers placed and cabled by
 * hand around a switch, everything else typed.
 *
 * <ul>
 * <li>{@code tls_web}: an in-game certificate authority, a certificate, HTTPS;
 * <li>{@code repo_kinds}: a library and an app that depends on it, from a
 *     signed repository;
 * <li>{@code repo_programs}: a program (a new command) and a source package;
 * <li>{@code repo_mirror}: a data-center mirror of a chosen part of an HTTPS
 *     repository.
 * </ul>
 *
 * The scenarios against the official repository's backend are in
 * {@link RepoBackendScenarios}.
 */
public final class RepoScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String PROMPT = PlayerKit.PROMPT;
    static final BlockPos SW = new BlockPos(4, 2, 4);

    /**
     * Up to four computers around a switch, the way {@code dns_lab} lays them
     * out: north, east, south, west. {@code hosts} are (name, address) pairs.
     */
    static void lan(Scenario.Builder b, String... hosts) {
        Direction[] faces = {Direction.DOWN, Direction.EAST, Direction.SOUTH, Direction.WEST};
        b.switchNode("sw", SW);
        for (int i = 0; i < hosts.length / 2; i++) {
            String name = hosts[2 * i], ip = hosts[2 * i + 1];
            Direction f = faces[i];
            switch (i) {
                case 0 -> {
                    b.host(name, SW.offset(0, -1, -3), ip);
                    b.link("sw-" + name, "sw", f, name, Direction.DOWN,
                            Scenario.Path.from(SW.relative(Direction.DOWN)).go(Direction.DOWN, 1).go(Direction.NORTH, 3));
                }
                case 1 -> {
                    b.host(name, SW.offset(4, -1, 0), ip);
                    b.link("sw-" + name, "sw", f, name, Direction.DOWN,
                            Scenario.Path.from(SW.relative(Direction.EAST)).go(Direction.EAST, 1).go(Direction.DOWN, 2).go(Direction.EAST, 2));
                }
                case 2 -> {
                    b.host(name, SW.offset(0, -1, 4), ip);
                    b.link("sw-" + name, "sw", f, name, Direction.DOWN,
                            Scenario.Path.from(SW.relative(Direction.SOUTH)).go(Direction.SOUTH, 1).go(Direction.DOWN, 2).go(Direction.SOUTH, 2));
                }
                default -> {
                    b.host(name, SW.offset(-4, -1, 0), ip);
                    b.link("sw-" + name, "sw", f, name, Direction.DOWN,
                            Scenario.Path.from(SW.relative(Direction.WEST)).go(Direction.WEST, 1).go(Direction.DOWN, 2).go(Direction.WEST, 2));
                }
            }
        }
        b.note("Setup (done for you): the computers, a switch, a cable from each computer's bottom face to the switch.");
        b.note("The switch: its ports as plain L2 ports in VLAN 1");
        b.send("sw", "switch on").expect("sw", "Switch started", "switch running");
        b.send("sw", "switch");
        for (int i = 0; i < hosts.length / 2; i++) {
            String p = b.eth("sw", faces[i]);
            b.send("sw", "interface " + p, "no routing");
            b.expect("sw", "converted to L2", p + " is an L2 port");
            b.send("sw", "exit");
        }
        b.configureHosts();
    }

    /**
     * Write text files on {@code node} with the shell: {@code mkdir -p} for each
     * folder, then one {@code echo '<line>' >> file} per line (the first with
     * {@code >}). Quicker than a Python session for a few short files; no line
     * may hold a single quote.
     */
    static void writeFiles(Scenario.Builder b, String node, Map<String, String> files) {
        java.util.Set<String> made = new java.util.HashSet<>();
        for (var f : files.entrySet()) {
            String path = f.getKey();
            String dir = path.substring(0, path.lastIndexOf('/'));
            if (!dir.isEmpty() && made.add(dir)) {
                b.send(node, "mkdir -p " + dir);
                b.expect(node, PROMPT, node + ": " + dir);
            }
            boolean first = true;
            for (String line : f.getValue().split("\n", -1)) {
                if (line.contains("'")) throw new IllegalArgumentException("writeFiles: a single quote in " + path);
                // One command at a time: the shell takes the next line once it is back at the prompt.
                b.send(node, "echo '" + line + "' " + (first ? ">" : ">>") + " " + path);
                b.expect(node, PROMPT, node + ": " + path);
                first = false;
            }
        }
        // The last line of the last file is on disk once the shell is back.
        String last = null;
        for (String p : files.keySet()) last = p;
        if (last != null) {
            String[] lines = files.get(last).split("\n", -1);
            String tail = lines[lines.length - 1];
            b.send(node, "cat " + last);
            b.expect(node, "^" + Pattern.quote(tail) + "$", node + ": the files are written (" + files.size() + ")");
            b.expect(node, PROMPT, node + ": shell");
        }
    }

    /** One shell command and its expected line, then the prompt. */
    static void run(Scenario.Builder b, String node, String command, String re, String what) {
        b.send(node, command);
        b.expect(node, re, what);
        b.expect(node, PROMPT, node + ": shell");
    }

    /** As {@link #run}, failing at once when {@code failRe} shows instead. */
    static void run(Scenario.Builder b, String node, String command, String re, String what, String failRe) {
        b.send(node, command);
        b.expect(node, re, what, failRe);
        b.expect(node, PROMPT, node + ": shell");
    }

    /** {@code node} becomes an authority, starts {@code cad}, and gets itself a certificate for {@code ip}. */
    static void authority(Scenario.Builder b, String node, String name) {
        run(b, node, "cad init " + name, "^This computer is the authority \"" + Pattern.quote(name) + "\"", node + " is an authority", "^cad: ");
        b.send(node, "cad &");
        b.expect(node, "Certificate authority \"" + Pattern.quote(name) + "\" listening on port 8555", "cad is running", "^cad: ");
    }

    // ------------------------------------------------------------------ tls_web

    static final String CA_IP = "10.0.9.1", WEB_IP = "10.0.9.2", TLS_CLIENT_IP = "10.0.9.3";

    /**
     * {@code tls_web} (docs/TLS.md): computer {@code ca} becomes a certificate
     * authority ({@code cad init}, {@code cad}). Computer {@code web} trusts it,
     * gets a certificate for its address ({@code cert request}: the authority
     * checks back on port 80, which {@code cert} answers itself) and serves a
     * page over HTTPS ({@code httpd 443 --tls}). Computer {@code client}
     * fetches it with {@code curl https://...} once it trusts the authority
     * too. Controls: before a certificate exists {@code httpd --tls} says how
     * to get one; the authority refuses a certificate for an address another
     * computer answers at; a client that trusts no authority is refused (and
     * {@code curl -k} shows the encryption itself works); a name the
     * certificate is not for is refused; after {@code cert untrust} the same
     * server is refused again.
     */
    static Scenario tlsWeb() {
        var b = Scenario.builder("tls_web",
                        "an in-game certificate authority (cad) signs a certificate for a web server (cert), which serves HTTPS (httpd --tls);"
                                + " a client reaches it with curl only once it trusts the authority")
                .asPlayer()
                .timeLimit(58_000);
        lan(b, "ca", CA_IP + "/24", "web", WEB_IP + "/24", "client", TLS_CLIENT_IP + "/24");

        b.note("ca: become an authority and start answering");
        run(b, "ca", "cad", "^cad: this computer is not an authority yet", "control: no authority before cad init");
        authority(b, "ca", "Lab Authority");

        b.note("web: a page to serve; control: HTTPS needs a certificate first");
        writeFiles(b, "web", Map.of("/srv/www/hello.txt", "hello-over-tls"));
        run(b, "web", "httpd 443 /srv/www --tls", "get a certificate first: cert request", "control: httpd --tls has no certificate yet");
        b.note("web: control - the authority only signs for a computer that answers at the name");
        run(b, "web", "cert request " + TLS_CLIENT_IP + " --ca " + CA_IP, "^cert: the authority refused: 10\\.0\\.9\\.3",
                "control: a certificate for another computer's address is refused", "Certificate installed");
        b.note("web: trust the authority, get the certificate, serve HTTPS");
        run(b, "web", "cert trust " + CA_IP, "^Now trusting \"Lab Authority\"", "web trusts the authority", "^cert: ");
        run(b, "web", "cert request " + WEB_IP, "^Certificate installed: /etc/ssl/server\\.crt", "the authority signed web's certificate", "^cert: ");
        b.expect("ca", "signed 10\\.0\\.9\\.2 until", "cad logged the certificate");
        run(b, "web", "cert show", "signed by  Lab Authority", "cert show names the authority");
        b.send("web", "httpd 443 /srv/www --tls &");
        b.expect("web", "HTTPS server listening on port 443, serving /srv/www", "httpd serves HTTPS", "^httpd: ");

        b.note("client: control - it trusts no authority, so the server is refused; -k encrypts without checking");
        run(b, "client", "curl https://" + WEB_IP + "/hello.txt", "trusts no in-game authority", "control: refused without a trusted authority",
                "^hello-over-tls$");
        run(b, "client", "curl -k https://" + WEB_IP + "/hello.txt", "^hello-over-tls$", "-k fetches it unchecked");
        b.note("client: trust the authority, fetch the page");
        run(b, "client", "cert trust " + CA_IP, "^Now trusting \"Lab Authority\"", "client trusts the authority", "^cert: ");
        b.send("client", "curl -v https://" + WEB_IP + "/hello.txt");
        b.expect("client", "TLSv1\\.3 connection to 10\\.0\\.9\\.2", "a TLS 1.3 connection");
        b.expect("client", "^hello-over-tls$", "the page arrived over HTTPS", "^curl: ");
        b.expect("client", PROMPT, "client: shell");
        b.expect("web", "GET /hello\\.txt", "httpd logged the request");

        b.note("client: control - a name the certificate is not for");
        writeFiles(b, "client", Map.of("/etc/hosts", WEB_IP + " shop.lab"));
        run(b, "client", "curl https://shop.lab/hello.txt", "certificate is for another name", "control: the wrong name is refused", "^hello-over-tls$");
        b.note("client: control - stop trusting the authority");
        run(b, "client", "cert untrust Lab Authority", "^No longer trusting \"Lab Authority\"", "authority removed");
        run(b, "client", "curl https://" + WEB_IP + "/hello.txt", "trusts no in-game authority", "control: refused again", "^hello-over-tls$");
        return b.build();
    }

    // ------------------------------------------------------------------ packages

    static final String ORIGIN_IP = "10.0.10.1", DC_IP = "10.0.10.2", PKG_CLIENT_IP = "10.0.10.3";

    static final String GEO_TOML = "id = \"dev.lab.geo\"\nname = \"Geometry\"\nversion = \"1.2.0\"\nkind = \"library\"\nprovides = [\"geo\"]";
    static final String GEO_PY = "def area(w, h):\n    return w * h";
    static final String GEO_LUA = "return { area = function(w, h) return w * h end }";
    static final String AREA_TOML = "id = \"dev.lab.area\"\nname = \"Area\"\nversion = \"1.0.0\"\nentry = \"main.py\"\ndepends = [\"dev.lab.geo >= 1.0.0\"]";
    static final String AREA_PY = "import geo\nprint(\"area is\", geo.area(6, 7))";
    static final String SHOUT_TOML = "id = \"dev.lab.shout\"\nname = \"Shout\"\nversion = \"1.0.0\"\nkind = \"program\"\nbin = [\"shout.wasm\"]";
    static final String NOTES_TOML = "id = \"dev.lab.notes\"\nname = \"Notes\"\nversion = \"0.1.0\"\nkind = \"source\"";

    /** The package sources a player would write: {@code which} of geo, area, shout, notes. */
    static Map<String, String> sources(String... which) {
        Map<String, String> f = new LinkedHashMap<>();
        for (String w : which) {
            switch (w) {
                case "geo" -> {
                    f.put("/src/geo/app.toml", GEO_TOML);
                    f.put("/src/geo/python/geo.py", GEO_PY);
                    f.put("/src/geo/lua/geo.lua", GEO_LUA);
                }
                case "area" -> {
                    f.put("/src/area/app.toml", AREA_TOML);
                    f.put("/src/area/main.py", AREA_PY);
                }
                case "shout" -> f.put("/src/shout/app.toml", SHOUT_TOML);
                default -> {
                    f.put("/src/notes/app.toml", NOTES_TOML);
                    f.put("/src/notes/blink.py", "print(\"blink\")");
                }
            }
        }
        return f;
    }

    /** The key {@code pkg keygen} printed on {@code node}'s screen (a player reads it off and types it). */
    static String keyOn(ScenarioRun r, String node) {
        Matcher m = Pattern.compile("(?m)^([0-9a-f]{64})\\s*$").matcher(r.screen(node));
        String last = null;
        while (m.find()) last = m.group(1);
        return last;
    }

    /** {@code node} builds and signs {@code names} into /srv/repo. A program is any .wasm: the built-in echo stands in for one the player compiled. */
    static void buildRepo(Scenario.Builder b, String node, String... names) {
        run(b, node, "pkg keygen", "^Public key \\(give it to people who pin your repository\\):", "a signing key", "^pkg: ");
        b.await(r -> keyOn(r, node) != null ? null : "no key on the screen", "the public key is on the screen", 5_000);
        b.mutate(r -> r.note("key", keyOn(r, node)), "(the player notes the public key down)");
        for (String n : names) {
            if (n.equals("shout")) run(b, node, "cp /server-bin/echo.wasm /src/shout/shout.wasm", PROMPT, "a program to package (a copy of echo)");
            run(b, node, "pkg build /src/" + n + " /srv/repo --sign /etc/pkg/signing.key", "^Signed with key [0-9a-f]{64}", n + " built and signed", "^pkg: ");
        }
    }

    /**
     * {@code repo_kinds} (docs/REPO.md): computer {@code origin} writes a
     * library (a Python and a Lua module) and an app that depends on it, signs
     * them ({@code pkg keygen}, {@code pkg build --sign}) and serves them
     * ({@code pkg serve}). Computer {@code client} adds the repository with
     * the key pinned and installs the app: its library comes first, and both
     * {@code import geo} in Python and {@code require("geo")} in Lua find it.
     * Controls: the library cannot be removed while the app needs it;
     * {@code pkg autoremove} takes it once the app is gone, and the import
     * fails again.
     */
    static Scenario repoKinds() {
        var b = Scenario.builder("repo_kinds",
                        "a signed repository with a library and an app that needs it; a client installs the app (the library comes first), imports the"
                                + " library in Python and Lua, and cannot remove the library while the app needs it")
                .asPlayer()
                .timeLimit(58_000);
        lan(b, "origin", ORIGIN_IP + "/24", "client", PKG_CLIENT_IP + "/24");

        b.note("origin: write the packages, sign and serve them; client: two one-line scripts that use the library");
        writeFiles(b, "origin", sources("geo", "area"));
        Map<String, String> scripts = new LinkedHashMap<>();
        scripts.put("/usegeo.py", "import geo\nprint(\"python says\", geo.area(2, 3))");
        scripts.put("/usegeo.lua", "print(\"lua says \" .. require(\"geo\").area(4, 5))");
        writeFiles(b, "client", scripts);
        buildRepo(b, "origin", "geo", "area");
        b.send("origin", "pkg serve /srv/repo 8080 &");
        b.expect("origin", "Serving 2 packages from /srv/repo on port 8080", "the repository is up", "^pkg: ");

        b.note("client: add the repository with its key pinned; the app brings its library");
        b.sendFn("client", r -> "pkg repo add lab http://" + ORIGIN_IP + ":8080/ --key " + r.noted("key"),
                "pkg repo add lab http://" + ORIGIN_IP + ":8080/ --key <the public key from origin's screen>");
        b.expect("client", "^Added lab .*: 2 packages", "index fetched", "^pkg: |not fetched");
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "pkg search geo", "^dev\\.lab\\.geo 1\\.2\\.0  Geometry  \\[library, lab, signed\\]", "the library is listed with its kind");
        b.send("client", "pkg install dev.lab.area");
        b.expect("client", "^Installed dev\\.lab\\.geo 1\\.2\\.0 \\(needed by dev\\.lab\\.area\\)", "the dependency came first", "^pkg: ");
        b.expect("client", "^Installed Area 1\\.0\\.0 \\(dev\\.lab\\.area\\) - verified publisher", "the app is installed, signed by the pinned key", "^pkg: ");
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "pkg list", "^dev\\.lab\\.geo 1\\.2\\.0  Geometry  \\[library\\]", "pkg list names the library's kind");
        run(b, "client", "lua /usegeo.lua", "^lua says 20$", "Lua requires the installed library", "module 'geo' not found");
        run(b, "client", "python /usegeo.py", "^python says 6$", "Python imports the installed library", "Error");

        b.note("client: control - the library is still needed; remove the app and it goes with autoremove");
        run(b, "client", "pkg remove dev.lab.geo", "^pkg: dev\\.lab\\.geo is needed by dev\\.lab\\.area; remove that first", "control: removal refused");
        run(b, "client", "pkg remove dev.lab.area", "^Removed dev\\.lab\\.area", "the app is removed");
        run(b, "client", "pkg autoremove", "^Removed dev\\.lab\\.geo \\(nothing needs it any more\\)", "the library nobody needs is removed");
        run(b, "client", "lua /usegeo.lua", "module 'geo' not found", "control: the module is gone with the library", "^lua says");
        return b.build();
    }

    /**
     * {@code repo_programs} (docs/REPO.md): computer {@code origin} writes a
     * program and a source package, signs and serves them. Computer
     * {@code client}, with the key pinned, installs the program: {@code pkg}
     * says it is not sandboxed, and its command works in the shell; the
     * source package lands in {@code /home/src}. Controls: removed, the
     * command is gone; the same program from the repository added without a
     * key is refused, since nobody vouches for it, and installs only with
     * {@code --unverified}. (That a package cannot take a system command's
     * name is covered by {@code cargo test -p pkg} and simulator scenario 51.)
     */
    static Scenario repoPrograms() {
        var b = Scenario.builder("repo_programs",
                        "a signed repository with a program and a source package; a client installs the program (a new shell command, not sandboxed)"
                                + " and the source package (files in /home/src); an unverified program is refused")
                .asPlayer()
                .timeLimit(58_000);
        lan(b, "origin", ORIGIN_IP + "/24", "client", PKG_CLIENT_IP + "/24");

        b.note("origin: write the packages, sign and serve them");
        writeFiles(b, "origin", sources("shout", "notes"));
        buildRepo(b, "origin", "shout", "notes");
        b.send("origin", "pkg serve /srv/repo 8080 &");
        b.expect("origin", "Serving 2 packages from /srv/repo on port 8080", "the repository is up", "^pkg: ");

        b.note("client: a program is a new command; a source package is files in /home/src");
        run(b, "client", "shout before", "(?i)not found|unknown command|no such", "control: there is no shout command yet");
        b.sendFn("client", r -> "pkg repo add lab http://" + ORIGIN_IP + ":8080/ --key " + r.noted("key"),
                "pkg repo add lab http://" + ORIGIN_IP + ":8080/ --key <the public key from origin's screen>");
        b.expect("client", "^Added lab .*: 2 packages", "index fetched", "^pkg: |not fetched");
        b.expect("client", PROMPT, "client: shell");
        b.send("client", "pkg install dev.lab.shout");
        b.expect("client", "^Note: dev\\.lab\\.shout is a program\\. Unlike an app it is not sandboxed", "pkg warns that a program is not sandboxed");
        b.expect("client", "^New commands: shout", "the command is installed", "^pkg: ");
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "shout it-works", "^it-works$", "the new command runs");
        run(b, "client", "pkg install dev.lab.notes", "^Its files are in /home/src/dev\\.lab\\.notes/", "the source package is unpacked", "^pkg: ");
        run(b, "client", "cat /home/src/dev.lab.notes/blink.py", "^print\\(\"blink\"\\)$", "its code is in /home/src");

        b.note("client: control - removed, the command is gone");
        run(b, "client", "pkg remove dev.lab.shout", "^Removed dev\\.lab\\.shout", "program removed");
        run(b, "client", "shout after", "(?i)not found|unknown command|no such", "control: the command went with the package");
        b.note("client: control - the same program from the repository with no pinned key: nobody vouches for it");
        run(b, "client", "pkg repo add lab http://" + ORIGIN_IP + ":8080/", "^Added lab ", "the repository again, without a key");
        run(b, "client", "pkg install dev.lab.shout", "whole computer in reach", "control: an unverified program is refused", "^New commands");
        run(b, "client", "pkg install dev.lab.shout --unverified", "^New commands: shout", "--unverified installs it anyway", "^pkg: ");
        return b.build();
    }

    /**
     * {@code repo_mirror} (docs/REPO.md, "Data center repositories"): computer
     * {@code origin} is a repository over HTTPS (its own authority, a
     * certificate, {@code pkg serve --tls}) with a library, an app that needs
     * it and a program. Computer {@code dc}, the data center, trusts the
     * authority and mirrors only the app ({@code pkg mirror --only}; the
     * library it needs comes along), then offers that on the game's network
     * in plain HTTP ({@code pkg serve}). Computer {@code client} pins the
     * origin's key and installs from the data center: the copy verifies as the
     * original would. Controls: before {@code dc} trusts the authority the
     * origin is refused and {@code pkg online} says why; a second mirror run
     * downloads nothing; the program the data center did not choose is not
     * offered to the client.
     */
    static Scenario repoMirror() {
        var b = Scenario.builder("repo_mirror",
                        "an HTTPS repository (pkg serve --tls with an in-game certificate); a data center mirrors a chosen part of it (pkg mirror --only)"
                                + " and serves that; a client installs from the data center with the origin's key pinned")
                .asPlayer()
                .timeLimit(58_000);
        lan(b, "origin", ORIGIN_IP + "/24", "dc", DC_IP + "/24", "client", PKG_CLIENT_IP + "/24");

        b.note("origin: three packages, signed; an authority and a certificate; the repository over HTTPS");
        writeFiles(b, "origin", sources("geo", "area", "shout"));
        buildRepo(b, "origin", "geo", "area", "shout");
        authority(b, "origin", "Origin Authority");
        run(b, "origin", "cert request " + ORIGIN_IP + " --ca " + ORIGIN_IP, "^Certificate installed", "origin has a certificate for its address", "^cert: ");
        b.send("origin", "pkg serve /srv/repo --tls &");
        b.expect("origin", "Serving 3 packages from /srv/repo on port 8443 \\(HTTPS\\)", "the repository is up over HTTPS", "^pkg: ");

        b.note("dc: control - the origin's certificate means nothing until its authority is trusted");
        b.sendFn("dc", r -> "pkg repo add up https://" + ORIGIN_IP + ":8443/ --key " + r.noted("key"),
                "pkg repo add up https://" + ORIGIN_IP + ":8443/ --key <the public key from origin's screen>");
        b.expect("dc", "index not fetched yet: .*trusts no in-game authority", "control: the origin is refused", "packages$");
        b.expect("dc", PROMPT, "dc: shell");
        run(b, "dc", "pkg online up", "trusts no in-game authority", "control: pkg online names the certificate as the problem");
        b.note("dc: trust it, mirror the app (its library comes along), serve the copy");
        run(b, "dc", "cert trust " + ORIGIN_IP, "^Now trusting \"Origin Authority\"", "dc trusts the authority", "^cert: ");
        run(b, "dc", "pkg online up", "^up  https://10\\.0\\.10\\.1:8443/  online", "the origin answers now");
        run(b, "dc", "pkg mirror up /srv/mirror --only dev.lab.area", "^2 copied, 0 already there\\.", "the app and its library were copied", "^pkg: ");
        run(b, "dc", "pkg mirror up /srv/mirror --only dev.lab.area", "^0 copied, 2 already there\\.", "control: a second run downloads nothing", "^pkg: ");
        b.send("dc", "pkg serve /srv/mirror 8080 &");
        b.expect("dc", "Serving 2 packages from /srv/mirror on port 8080\\.", "the data center offers its two packages", "^pkg: ");
        b.expect("dc", "dev\\.lab\\.area 1\\.0\\.0 \\(signed\\)", "the copies kept the origin's signatures");

        b.note("client: the data center's repository, with the origin's key pinned");
        b.sendFn("client", r -> "pkg repo add dc http://" + DC_IP + ":8080/ --key " + r.noted("key"),
                "pkg repo add dc http://" + DC_IP + ":8080/ --key <the public key from origin's screen>");
        b.expect("client", "^Added dc .*: 2 packages", "the client sees the two packages", "^pkg: |not fetched");
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "pkg install dev.lab.shout", "^pkg: dev\\.lab\\.shout is not in any repository", "control: what the data center did not choose is not offered");
        b.send("client", "pkg install dev.lab.area");
        b.expect("client", "^Installed Area 1\\.0\\.0 \\(dev\\.lab\\.area\\) - verified publisher", "installed from the mirror, verified against the origin's key", "^pkg: ");
        b.expect("client", PROMPT, "client: shell");
        b.expect("dc", "GET /dev\\.lab\\.area-1\\.0\\.0\\.ecmpkg", "the data center served it");
        run(b, "client", "python /apps/dev.lab.area/main.py", "^area is 42$", "the app runs with the mirrored library", "Error");
        return b.build();
    }

    static {
        for (Scenario s : new Scenario[] {tlsWeb(), repoKinds(), repoPrograms(), repoMirror()}) ALL.put(s.name, s);
    }

    private RepoScenarios() {}
}
//?}
