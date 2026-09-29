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
    [string]$McVersion = "26.1",    # JUnit/GameTests run on 26.1 (1.21.1 needs Sable at runtime)
    [switch]$NoStage                # skip rebuilding/staging the WASM
)

$ErrorActionPreference = "Stop"
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
    $cmd = ".\gradlew.bat :$McVersion`:runGameTestServer -PgameTestNamespaces=$ns -PtestRunDir=$runDir --console=plain"
    $log = Join-Path $out "gametest.log"
    $code = Run-Logged $cmd $log
    $text = Get-Content $log -Raw
    foreach ($n in $GameTests) {
        $m = [regex]::Match($text, "Running test environment '$n`:[^']+' batch \d+ \((\d+) tests?\)")
        $expected = if ($m.Success) { [int]$m.Groups[1].Value } else { 0 }
        $marker = "ECM_" + ($n -replace "^ecm_", "").ToUpper() + "_TEST_PASS"
        $names = [regex]::Matches($text, "$marker (\S+)") | ForEach-Object { $_.Groups[1].Value }
        $unique = @($names | Sort-Object -Unique)
        $ok = ($expected -gt 0) -and ($unique.Count -eq $expected) -and ($names.Count -eq $unique.Count)
        Add-Step "gametest:$n" $cmd $log $ok $expected $unique.Count "markers=$($unique -join ',')"
    }
    $allPassed = [regex]::IsMatch($text, "All \d+ required tests passed")
    if (-not $allPassed -or $code -ne 0) {
        $result.status = "FAIL"
        $result.errors += "gametest : server did not report all required tests passed (exit $code)"
    }
    Scan-Log "gametest" $log
}

# ---------------------------------------------------------------- scenarios
if ($Scenarios.Count -gt 0) {
    foreach ($f in $Scenarios) {
        $cmd = "cd rust && cargo test -p terminal-simulator --release $f"
        $log = Join-Path $out "scenario-$($f -replace '[^\w-]','_').log"
        $code = Run-Logged $cmd $log
        $c = Cargo-Counts $log
        $ok = ($code -eq 0) -and ($c.failed -eq 0) -and ($c.passed -gt 0)
        Add-Step "scenario:$f" $cmd $log $ok $null $c.passed "passed=$($c.passed) failed=$($c.failed) exit=$code"
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
