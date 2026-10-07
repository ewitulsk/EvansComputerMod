<#
.SYNOPSIS
  Targeted test runner for EvansComputerMod (see TESTING.md).

.DESCRIPTION
  Runs only what you ask for, collects logs, and writes a receipt:
    artifacts/<area>-yyyyMMdd-HHmmss/result.json  (+ one log per step)

  Pass/fail is decided from what actually ran, never from exit codes alone:
    - cargo : every "test result:" line ok, and > 0 tests ran
    - JUnit : the XML reports of the requested classes exist, > 0 tests,
              no failures / errors / skips
    - GameTests : one ECM_<NS>_TEST_PASS marker per test in each namespace
              (no duplicates), and "All N required tests passed" in the log
    - scenarios : like cargo, for the simulator's scenario tests
  Logs are scanned for panics / crashes / uncaught exceptions.

.EXAMPLE
  scripts/Test.ps1 -Area kernel -Rust terminal-os
  scripts/Test.ps1 -Area ssh -JUnit KernelHostIntegrationTest -GameTests ecm_network
  scripts/Test.ps1 -Area switch -Rust ecm-bridge,terminal-os -Scenarios switch_
#>
param(
    [string]$Area = "adhoc",
    [string[]]$Rust = @(),          # cargo packages, e.g. ecm-net, ecm-bridge, terminal-os
    [string[]]$JUnit = @(),         # simple class names, e.g. KernelHostIntegrationTest
    [string[]]$GameTests = @(),     # namespaces, e.g. ecm_network
    [string[]]$Scenarios = @(),     # simulator scenario filters (cargo test name filters)
    [string]$McVersion = "1.21.1",  # GameTests: 1.21.1 (ecm_switch, ecm_sync, ecm_periph, ecm_sensor, ecm_screen) or 26.1 (ecm_network, ecm_switch); JUnit needs 26.1
    [switch]$NoStage,               # skip rebuilding/staging the WASM
    [switch]$NoCreate               # 1.21.1 GameTests: don't load Create
)

$ErrorActionPreference = "Stop"

# `powershell -File` passes "a,b" as one string; accept both forms.
function Split-List($v) { @($v | ForEach-Object { $_ -split "," } | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
$Rust = Split-List $Rust
$JUnit = Split-List $JUnit
$GameTests = Split-List $GameTests
$Scenarios = Split-List $Scenarios
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$out = Join-Path $root "artifacts\$Area-$stamp"
New-Item -ItemType Directory -Force $out | Out-Null
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

$result = [ordered]@{
    status    = "PASS"
    area      = $Area
    revision  = (git rev-parse --short HEAD).Trim()
    dirty     = [bool](git status --porcelain)
    started   = (Get-Date).ToString("o")
    steps     = @()
    errors    = @()
}

function Add-Step($name, $command, $log, $ok, $expected, $completed, $detail) {
    $script:result.steps += [ordered]@{
        name = $name; command = $command; log = (Split-Path -Leaf $log)
        ok = $ok; expected = $expected; completed = $completed; detail = $detail
    }
    if (-not $ok) {
        $script:result.status = "FAIL"
        $script:result.errors += "$name : $detail"
    }
}

function Run-Logged($command, $log) {
    # Run through cmd so native stderr doesn't become PowerShell errors.
    cmd /c "$command > `"$log`" 2>&1"
    return $LASTEXITCODE
}

function Scan-Log($name, $log) {
    $bad = Select-String -Path $log -Pattern "panicked at|FATAL|A fatal error|Crash report|Exception in thread" -SimpleMatch:$false
    if ($bad) {
        $script:result.errors += "$name : log contains crash/panic lines (first: $($bad[0].Line.Trim()))"
        $script:result.status = "FAIL"
    }
}

function Cargo-Counts($log) {
    $passed = 0; $failed = 0; $lines = 0
    foreach ($m in (Select-String -Path $log -Pattern "test result: (\w+)\. (\d+) passed; (\d+) failed")) {
        $lines++
        $passed += [int]$m.Matches[0].Groups[2].Value
        $failed += [int]$m.Matches[0].Groups[3].Value
    }
    return @{ passed = $passed; failed = $failed; lines = $lines }
}

# ---------------------------------------------------------------- stage WASM
if (-not $NoStage -and ($JUnit.Count -gt 0 -or $GameTests.Count -gt 0 -or $Scenarios.Count -gt 0)) {
    # Prefer Git Bash; the `bash` on PATH is often the WSL launcher stub
    # (WindowsApps\bash.exe), which fails without a distro.
    $bash = $null
    foreach ($c in @("$env:ProgramFiles\Git\bin\bash.exe", "${env:ProgramFiles(x86)}\Git\bin\bash.exe")) {
        if (Test-Path $c) { $bash = $c; break }
    }
    if (-not $bash) {
        $bash = (Get-Command bash -All -ErrorAction SilentlyContinue |
                 Where-Object { $_.Source -notmatch "WindowsApps" } | Select-Object -First 1).Source
    }
    $log = Join-Path $out "stage-wasm.log"
    $cmd = "`"$bash`" scripts/stage-wasm.sh"
    $code = Run-Logged $cmd $log
    Add-Step "stage-wasm" $cmd $log ($code -eq 0) $null $null "exit $code"
}

# ---------------------------------------------------------------- cargo
if ($Rust.Count -gt 0) {
    $pkgs = ($Rust | ForEach-Object { "-p $_" }) -join " "
    $cmd = "cd rust && cargo test $pkgs"
    $log = Join-Path $out "cargo.log"
    $code = Run-Logged $cmd $log
    $c = Cargo-Counts $log
    $ok = ($code -eq 0) -and ($c.failed -eq 0) -and ($c.passed -gt 0)
    Add-Step "cargo" $cmd $log $ok $null $c.passed "passed=$($c.passed) failed=$($c.failed) exit=$code"
    Scan-Log "cargo" $log
}

# ---------------------------------------------------------------- JUnit
if ($JUnit.Count -gt 0) {
    $filters = ($JUnit | ForEach-Object { "--tests *.$_" }) -join " "
    $cmd = ".\gradlew.bat :$McVersion`:test $filters --console=plain"
    $log = Join-Path $out "junit.log"
    $code = Run-Logged $cmd $log
    $tests = 0; $bad = 0
    foreach ($cls in $JUnit) {
        $xml = Get-ChildItem "versions\$McVersion\build\test-results\test\TEST-*.$cls.xml" -ErrorAction SilentlyContinue |
               Where-Object { $_.LastWriteTime -gt [datetime]$result.started }
        if (-not $xml) { $bad++; $result.errors += "junit : no fresh report for $cls"; continue }
        Copy-Item $xml.FullName $out
        [xml]$r = Get-Content $xml.FullName
        $s = $r.testsuite
        $tests += [int]$s.tests
        $bad += [int]$s.failures + [int]$s.errors + [int]$s.skipped
    }
    $ok = ($code -eq 0) -and ($bad -eq 0) -and ($tests -gt 0)
    Add-Step "junit" $cmd $log $ok $null $tests "tests=$tests failures+errors+skips=$bad exit=$code"
}

# ---------------------------------------------------------------- GameTests
if ($GameTests.Count -gt 0) {
    $ns = $GameTests -join ","
    $runDir = "runs/gametest-$stamp"
    if ($McVersion -eq "1.21.1") {
        # The 1.21.1 mod implements Sable interfaces, so Sable must be loaded;
        # Create is loaded too so the Redstone Link module (ecm_periph) can be
        # tested against real Create links. Both go in the run's mods/ folder
        # (they bundle their own libraries via jar-in-jar). -NoCreate leaves
        # Create out, to check the mod still works without it.
        if (-not (Test-Path (Join-Path $root "libs\create-1.21.1-*.jar"))) {
            & "$env:ProgramFiles\Git\bin\bash.exe" scripts/fetch-libs.sh
        }
        $mods = Join-Path $root "$runDir\mods"
        New-Item -ItemType Directory -Force $mods | Out-Null
        Copy-Item (Join-Path $root "libs\sable-neoforge-1.21.1-*.jar") $mods
        if (-not $NoCreate) { Copy-Item (Join-Path $root "libs\create-1.21.1-*.jar") $mods }
    }
    $cmd = ".\gradlew.bat :$McVersion`:runGameTestServer -PgameTestNamespaces=$ns -PtestRunDir=$runDir --console=plain"
    $log = Join-Path $out "gametest.log"
    $code = Run-Logged $cmd $log
    $text = Get-Content $log -Raw
    foreach ($n in $GameTests) {
        # A namespace can span several environments/batches (ecm_switch runs one per scenario).
        $expected = 0
        # 26.1: "Running test environment 'ns:env' batch 0 (N tests)"; 1.21.1: "Running test batch 'ns.x' (N tests)".
        foreach ($m in [regex]::Matches($text, "Running test (?:environment '$n`:[^']+' batch \d+|batch '$n\.[^']*') \((\d+) tests?\)")) {
            $expected += [int]$m.Groups[1].Value
        }
        $marker = "ECM_" + ($n -replace "^ecm_", "").ToUpper() + "_TEST_PASS"
        $names = [regex]::Matches($text, "$marker (\S+)") | ForEach-Object { $_.Groups[1].Value }
        $unique = @($names | Sort-Object -Unique)
        $ok = ($expected -gt 0) -and ($unique.Count -eq $expected) -and ($names.Count -eq $unique.Count)
        Add-Step "gametest:$n" $cmd $log $ok $expected $unique.Count "markers=$($unique -join ',')"
    }
    # Screenshots written by tests (testing/ScreenCapture) go with the receipt.
    $shots = Join-Path $root "$runDir\screenshots"
    if (Test-Path $shots) { Copy-Item -Recurse $shots (Join-Path $out "screenshots") }
    $allPassed = [regex]::IsMatch($text, "All \d+ required tests passed")
    if (-not $allPassed -or $code -ne 0) {
        $result.status = "FAIL"
        $result.errors += "gametest : server did not report all required tests passed (exit $code)"
    }
    Scan-Log "gametest" $log
}

# ---------------------------------------------------------------- scenarios
if ($Scenarios.Count -gt 0) {
    # Filters match scenario file names (rust/simulator/scenarios/*.toml);
    # "all" runs every scenario.
    foreach ($f in $Scenarios) {
        $filter = if ($f -eq "all") { "" } else { $f }
        $cmd = "cd rust && cargo test -p terminal-simulator --release --test scenarios $filter"
        $log = Join-Path $out "scenario-$($f -replace '[^\w-]','_').log"
        $code = Run-Logged $cmd $log
        $m = Select-String -Path $log -Pattern "scenario result: (\d+) passed; (\d+) failed" | Select-Object -Last 1
        $passed = if ($m) { [int]$m.Matches[0].Groups[1].Value } else { 0 }
        $failed = if ($m) { [int]$m.Matches[0].Groups[2].Value } else { 0 }
        $ok = ($code -eq 0) -and ($failed -eq 0) -and ($passed -gt 0)
        Add-Step "scenario:$f" $cmd $log $ok $null $passed "passed=$passed failed=$failed exit=$code"
    }
}

if ($result.steps.Count -eq 0) {
    $result.status = "FAIL"
    $result.errors += "nothing was run (pass -Rust / -JUnit / -GameTests / -Scenarios)"
}
if ($result.status -eq "PASS" -and ($Rust.Count + $JUnit.Count + $GameTests.Count + $Scenarios.Count) -gt 0) {
    # A targeted run that passes is a DIAGNOSTIC_PASS for the areas it covered.
    $result.status = "DIAGNOSTIC_PASS"
}
$result.finished = (Get-Date).ToString("o")
$result | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 (Join-Path $out "result.json")

Write-Host ""
Write-Host "$($result.status)  ($Area)  -> $out"
foreach ($s in $result.steps) {
    $mark = if ($s.ok) { "ok  " } else { "FAIL" }
    Write-Host "  [$mark] $($s.name): $($s.detail)"
}
foreach ($e in $result.errors) { Write-Host "  ! $e" }
if ($result.status -eq "FAIL") { exit 1 } else { exit 0 }
