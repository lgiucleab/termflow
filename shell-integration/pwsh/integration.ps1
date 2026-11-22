# TermFlow shell integration for PowerShell 7 and Windows PowerShell 5.1. Emits OSC 133 / 6973 marks.
if ($Global:__TfLoaded) { return }
$Global:__TfLoaded = $true
$Global:__TfCmdRan = $false
$Global:__TfUserPrompt = $function:prompt

function Global:__TfOsc([string]$s) { return "$([char]27)]$s$([char]7)" }

function Global:prompt {
    # Must be first: capture the last command's status before anything else runs.
    $ok = $?
    $ec = if ($ok) { 0 } elseif ($LASTEXITCODE) { $LASTEXITCODE } else { 1 }
    $out = ""
    if ($Global:__TfCmdRan) { $out += __TfOsc "133;D;$ec"; $Global:__TfCmdRan = $false }
    if ($PWD.Provider.Name -eq "FileSystem") { $out += __TfOsc "6973;cwd=$($PWD.ProviderPath)" }
    $branch = git symbolic-ref --short HEAD 2>$null
    $out += __TfOsc "6973;git_branch=$branch"
    $out += __TfOsc "133;A"
    $out += & $Global:__TfUserPrompt
    $out += __TfOsc "133;B"
    $global:LASTEXITCODE = $ec
    return $out
}

if (Get-Module -Name PSReadLine) {
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
        $line = $null; $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($line))
        [Console]::Write((__TfOsc "6973;cmd=$b64") + (__TfOsc "133;C"))
        $Global:__TfCmdRan = $true
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
