//! A small styled site: what a player writes with `edit` in the
//! `browser_local_site` scenario, and what the golden images show (a flex
//! navigation bar, cards that wrap, and a phone layout from `@media`).

pub const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Alex's Workshop</title><link rel="stylesheet" href="style.css"></head>
<body>
<nav class="bar"><b class="logo">Alex's Workshop</b><a href="index.html">Home</a><a href="parts.html">Parts</a></nav>
<main>
<h1>Redstone and fiber</h1>
<p class="lead">Builds and parts lists from the house by the river.</p>
<div class="cards">
<div class="card"><h3>Sorter</h3><p>Hoppers, comparators and a lot of patience.</p></div>
<div class="card"><h3>Fiber tap</h3><p>A patch panel on the span to village 4.</p></div>
<div class="card"><h3>Weather</h3><p>Lidar on the roof, numbers in Python.</p></div>
</div>
</main>
<footer>Served by httpd on my own computer.</footer>
</body></html>
"#;

pub const STYLE_CSS: &str = r#"body { margin: 0; background: #f4f1ea; color: #2b2b2b }
.bar { display: flex; gap: 16px; align-items: center; padding: 8px 16px; background: #2f5f9e }
.bar a { color: #ffffff; text-decoration: none }
.logo { color: #ffd166; margin-right: auto }
main { padding: 8px 16px }
.lead { color: #6b6b6b }
.cards { display: flex; flex-wrap: wrap; gap: 12px }
.card { flex: 1 1 160px; background: #ffffff; border: 1px solid #d8d2c4; border-radius: 6px; padding: 8px 12px }
.card h3 { margin: 0 0 4px 0; color: #2f5f9e }
footer { padding: 8px 16px; color: #8a8a8a; border-top: 1px solid #d8d2c4 }
@media (max-width: 480px) { .card { flex-basis: 100% } }
"#;
