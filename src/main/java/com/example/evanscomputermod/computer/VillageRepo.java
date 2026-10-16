package com.example.evanscomputermod.computer;

import java.io.IOException;
import java.io.InputStream;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.KeyFactory;
import java.security.KeyPairGenerator;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.security.Signature;
import java.security.interfaces.EdECPublicKey;
import java.security.spec.EdECPoint;
import java.security.spec.EdECPrivateKeySpec;
import java.security.spec.EdECPublicKeySpec;
import java.security.spec.NamedParameterSpec;
import java.util.*;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The Tech Village package repository (docs/TECH_VILLAGE_NETWORK.md "Apps"): a computer in the
 * chat village's Data Center runs {@code pkg serve /srv/repo 8080} over a real repository, the
 * Chat app's package (built with the mod by {@code scripts/stage-wasm.sh}: {@code chatapp
 * --source} + {@code pkg build}, run natively) signed with this world's repository key, an
 * ed25519 key derived from the seed. Every generated village computer and every Phone Store
 * phone has the repository in {@code /etc/pkg/repos} with that key pinned, so the App Store
 * shows its apps as "verified publisher". Plain Java: the formats are {@code ecm_pkg}'s
 * (docs/gui/APPS.md "Repositories", "Signing and trust"); {@code VillageRepoTest} checks the
 * signatures against {@code pkg}'s own vector.
 */
public record VillageRepo(int village, String publicKey, byte[] secret) {
    /** The name in {@code /etc/pkg/repos} (letters, digits, - and _: {@code ecm_pkg::valid_repo_name}). */
    public static final String NAME = "Tech-Village-Apps";
    /** Last octet of the package server on the data center LAN, and its port. */
    public static final int HOST = 21, PORT = 8080;
    /** The repository folder on the package server (pkg serve's argument). */
    public static final String DIR = "srv/repo";
    /** Where the mod's build puts the packages (wasm-bin/repo/, see the village_repo example). */
    static final String RESOURCES = "/wasm-bin/repo/";
    /** The repository list file and its header ({@code ecm_pkg::format_repos}). */
    public static final String REPOS_FILE = "etc/pkg/repos";
    static final String REPOS_HEADER = "# pkg repositories: <name> <url> [<pinned ed25519 key>]\n";

    /** This world's repository: the key from the seed, the server in the chat village. */
    public static VillageRepo of(long seed, int village) {
        byte[] secret = secretOf(seed);
        return new VillageRepo(village, publicKeyHex(secret), secret);
    }

    /** The 32-byte ed25519 seed: HMAC-SHA256(world seed, "ECM-TECHVILLAGE-REPO"). Server-side only. */
    public static byte[] secretOf(long seed) {
        try {
            var mac = javax.crypto.Mac.getInstance("HmacSHA256");
            byte[] k = new byte[8];
            for (int i = 0; i < 8; i++) k[i] = (byte) (seed >>> (56 - 8 * i));
            mac.init(new javax.crypto.spec.SecretKeySpec(k, "HmacSHA256"));
            return mac.doFinal("ECM-TECHVILLAGE-REPO".getBytes(StandardCharsets.US_ASCII));
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    public String address() {
        return "100." + (64 + village) + ".0." + HOST;
    }

    /** The repository URL as {@code pkg repo add} stores it. */
    public String url() {
        return "http://" + address() + ":" + PORT + "/";
    }

    /** {@code /etc/pkg/repos} with this repository and its pinned key (what {@code pkg repo add ... --key} writes). */
    public String reposFile() {
        return REPOS_HEADER + NAME + " " + url() + " " + publicKey + "\n";
    }

    /** {@code pkg serve} as the package server's services.cfg starts it (named for service discovery). */
    public static String serveCommand() {
        return "pkg serve /" + DIR + " " + PORT + " --name " + NAME + " &";
    }

    // ------------------------------------------------------------------ the server's files

    /** A package of the repository: its file name and bytes, id and version from its app.toml. */
    public record Package(String file, byte[] bytes, String id, String version) {
        public String sha256() {
            try {
                return CellPlan.hex(MessageDigest.getInstance("SHA-256").digest(bytes));
            } catch (Exception e) {
                throw new IllegalStateException(e);
            }
        }
    }

    private static volatile List<Package> packages;

    /** The packages the mod was built with (empty in a build without staged WASM). */
    public static List<Package> packages() {
        List<Package> p = packages;
        if (p != null) return p;
        List<Package> out = new ArrayList<>();
        try (InputStream in = VillageRepo.class.getResourceAsStream(RESOURCES + "packages.txt")) {
            if (in != null)
                for (String name : new String(in.readAllBytes(), StandardCharsets.UTF_8).split("\n")) {
                    name = name.trim();
                    if (name.isEmpty()) continue;
                    try (InputStream pin = VillageRepo.class.getResourceAsStream(RESOURCES + name)) {
                        if (pin == null) throw new IllegalStateException("missing repository package " + name);
                        out.add(read(name, pin.readAllBytes()));
                    }
                }
        } catch (IOException e) {
            throw new IllegalStateException("cannot read the repository packages", e);
        }
        packages = p = List.copyOf(out);
        return p;
    }

    /** Read id and version from a package's first member, app.toml (a ustar archive, docs/gui/APPS.md). */
    public static Package read(String file, byte[] bytes) {
        if (bytes.length < 512) throw new IllegalArgumentException(file + ": not a package");
        String name = new String(bytes, 0, 100, StandardCharsets.US_ASCII).replace("\0", "");
        if (!name.equals("app.toml")) throw new IllegalArgumentException(file + ": the first member is " + name);
        int size = Integer.parseInt(new String(bytes, 124, 11, StandardCharsets.US_ASCII).replace("\0", "").trim(), 8);
        String toml = new String(bytes, 512, size, StandardCharsets.UTF_8);
        return new Package(file, bytes, field(toml, "id"), field(toml, "version"));
    }

    private static String field(String toml, String key) {
        Matcher m = Pattern.compile("(?m)^\\s*" + key + "\\s*=\\s*\"([^\"]+)\"").matcher(toml);
        if (!m.find()) throw new IllegalArgumentException("app.toml has no " + key);
        return m.group(1);
    }

    /**
     * The package server's repository files, relative to its storage: every package with its
     * signature ({@code <file>.sig}) and {@code publisher.pub}, as {@code pkg build <dir> /srv/repo
     * --sign <key>} leaves them; {@code pkg serve} makes {@code index.json} from them. Also the
     * signing key in {@code /etc/pkg/signing.key} (the server is the publisher: {@code pkg build
     * ... --sign /etc/pkg/signing.key} publishes more verified apps there).
     */
    public Map<String, byte[]> serverFiles() {
        Map<String, byte[]> files = new LinkedHashMap<>();
        for (Package p : packages()) {
            files.put(DIR + "/" + p.file(), p.bytes());
            files.put(DIR + "/" + p.file() + ".sig", sign(secret, signedMessage(p.id(), p.version(), p.sha256())).getBytes(StandardCharsets.US_ASCII));
        }
        files.put(DIR + "/publisher.pub", publicKey.getBytes(StandardCharsets.US_ASCII));
        files.put("etc/pkg/signing.key", CellPlan.hex(secret).getBytes(StandardCharsets.US_ASCII));
        return files;
    }

    // ------------------------------------------------------------------ ed25519 (ecm_pkg::sign)

    /** What a signature covers ({@code ecm_pkg::package::signed_message}). */
    public static byte[] signedMessage(String id, String version, String sha256) {
        return ("ecmpkg-sig-v1\n" + id + "\n" + version + "\n" + sha256 + "\n").getBytes(StandardCharsets.UTF_8);
    }

    /** The public key (hex) of a 32-byte ed25519 seed, as {@code ecm_pkg::sign::keypair_from_seed}. */
    public static String publicKeyHex(byte[] seed) {
        try {
            KeyPairGenerator g = KeyPairGenerator.getInstance("Ed25519");
            g.initialize(NamedParameterSpec.ED25519, new FixedRandom(seed));
            EdECPoint pt = ((EdECPublicKey) g.generateKeyPair().getPublic()).getPoint();
            return CellPlan.hex(encode(pt));
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** RFC 8032 encoding: y little-endian, the top bit of the last byte is x's parity. */
    private static byte[] encode(EdECPoint p) {
        byte[] be = p.getY().toByteArray();
        byte[] out = new byte[32];
        for (int i = 0; i < 32 && i < be.length; i++) out[i] = be[be.length - 1 - i];
        if (p.isXOdd()) out[31] |= (byte) 0x80;
        return out;
    }

    /** Signature hex over {@code msg} with a 32-byte seed ({@code ecm_pkg::sign::sign}). */
    public static String sign(byte[] seed, byte[] msg) {
        try {
            var key = KeyFactory.getInstance("Ed25519").generatePrivate(new EdECPrivateKeySpec(NamedParameterSpec.ED25519, seed));
            Signature s = Signature.getInstance("Ed25519");
            s.initSign(key);
            s.update(msg);
            return CellPlan.hex(s.sign());
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** Check a signature hex against a public key hex ({@code ecm_pkg::sign::verify}). */
    public static boolean verify(String publicHex, byte[] msg, String sigHex) {
        try {
            byte[] pk = unhex(publicHex), sig = unhex(sigHex);
            if (pk.length != 32 || sig.length != 64) return false;
            boolean xOdd = (pk[31] & 0x80) != 0;
            byte[] be = new byte[32];
            for (int i = 0; i < 32; i++) be[i] = pk[31 - i];
            be[0] &= 0x7f;
            var spec = new EdECPublicKeySpec(NamedParameterSpec.ED25519, new EdECPoint(xOdd, new BigInteger(1, be)));
            Signature s = Signature.getInstance("Ed25519");
            s.initVerify(KeyFactory.getInstance("Ed25519").generatePublic(spec));
            s.update(msg);
            return s.verify(sig);
        } catch (Exception e) {
            return false;
        }
    }

    public static byte[] unhex(String s) {
        s = s.trim();
        if (s.length() % 2 != 0) return new byte[0];
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        return out;
    }

    /** Hands the key generator exactly the seed (ed25519 private keys are the 32 random bytes it draws). */
    private static final class FixedRandom extends SecureRandom {
        private final byte[] seed;

        FixedRandom(byte[] seed) {
            this.seed = seed.clone();
        }

        @Override
        public void nextBytes(byte[] bytes) {
            if (bytes.length != seed.length) throw new IllegalStateException("ed25519 wants " + bytes.length + " bytes");
            System.arraycopy(seed, 0, bytes, 0, bytes.length);
        }
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof VillageRepo r && r.village == village && r.publicKey.equals(publicKey);
    }

    @Override
    public int hashCode() {
        return Objects.hash(village, publicKey);
    }

    @Override
    public String toString() {
        return "VillageRepo[" + NAME + " " + url() + " key " + publicKey + "]";
    }
}
