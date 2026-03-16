/************************************************************************
 * @description AHK script preprocessor for build pipelines, maybe
 * @date 2026/02/07
 * @version 0.0.0
 ***********************************************************************/

#Requires AutoHotkey v2.0.0+

; TODO warn or throw for incompatible requirements e.g. bitness, version ranges with no overlap, generate #Requires from these statements?
; The #Requires directive is suprisingly complicated, though VerCompare should do most of the heavy lifting for us
; See https://www.autohotkey.com/docs/v2/lib/_Requires.htm#Remarks

; TODO Trim inline comments if --keep-comments is false
; Watch out for comments in string literals, continuation sections, etc

; Potential feature: Preprocessor flags?
; Take arg like --define DEBUG and in script respect
;   ;@ahkbuild-ignoreifdef DEBUG    - start ignore block if flag is defined
;   ;@ahkbuild-ignoreifndef DEBUG   - start ignore block if flag is not defined
;   ;@ahkbuild-define DEBUG         - define flag
;   ;@ahkbuild-undef DEBUG          - undefine flag
; We already have ignorebegin / ignoreend, can reuse ignore block logic
; Maybe get fancier - ;@ahkbuild-ignoreif BITNESS=64, ;@ahkbuild-ignoreif LEVEL<=1 take flags like --define BITNESS=32
; Allow "naked" ignores to be ifdef ;@ahkbuild-ignoreif DEBUG means "ignore if DEBUG is defined"
; Maybe also allow A_* variable resolution? --define PTRSIZE=%A_PtrSize%. Probably less useful since these will be
; resolved by the preprocessor and not the script itself

#Include <Extensions\StringExtensions>
#Include <argparse\ArgumentParser>
#Include <log4ahk\Log>
#Include <log4ahk\appenders\FileAppender>
#Include Lib\utils.ahk

;@Ahk2Exe-ConsoleApp
;@Ahk2Exe-SetCopyright Copyright 2026 Tao Beloney
;@Ahk2Exe-SetDescription AutoHotkey script preprocessor

; Example command lines (PowerShell):
;   AutoHotkey64.exe .\preprocess\include.ahk .\include.ahk .\include.out.ahk --overwrite --keep-comments --keep-empty 2>&1 | Write-Host
;   AutoHotkey64.exe .\preprocess\include.ahk "C:\Users\taorc\Documents\AutoHotkey\Lib\Extensions\BufferExtensions.ahk" "./test.out.ahk" --loglevel=TRACE 2>&1 | Write-Host

args := ParseCommandLine()

FileAppend("", args.loglocation)

Log.Configure(args.loglevel)
    .ToLogger(Log.Logger()
        .WithAppender(FileAppender(args.loglocation, 50))
        .WithAppender(ConsoleAppender().WithPattern("{Level}: {Message}"))
)

if(FileExist(args.output) && !args.overwrite) {
    Log.Fatal(Format(
        "Output file '{1}' already exists.`r`nSpecify --overwrite or -o to overwrite it if this is intentional.",
        args.output
    ))
    ExitApp(1)
}

if(!args.dryrun) {
    ; NOTE: this means outFile will be unset in a dry run
    outFile := FileOpen(args.output, "w-")
    OnError((*) => outFile.Close())
}
else {
    outFile := ""
}

OnError((thrown, mode) => (Log.LogMessage(mode == "ExitApp" ? Log.Level.FATAL : Log.Level.ERROR, thrown), 1))

includeMap := Map()
includeMap[args.input] := true
ParseInclude(args.input, outFile, includeMap, true)

summary := Format("Included {1} file(s):", includeMap.Count)
for (key, val in includeMap) {
    summary .= "`r`n" key
}

Log.Info(summary)

/**
 * Parses #Include statements for a script, outputting the result
 * @param {String} input the file to parse
 * @param {File} output the file to write to
 * @param {Map<String, Boolean>} includeMap map of absolute paths of include statements
 * @param {Boolean} keepComents true to keep comments, false to strip them
 */
ParseInclude(input, output, includeMap, isRoot := false, includeChain := []) {
    static includePattern := "ims)^\s?#Include(?'again'Again)?\s(?'ignore'\*i)?\s?(?'path'.*)\s?$"

    Log.Info(Format("Processing file '{1}'", input))

    normalizedInput := StrReplace(input, "/", "\")

    ; Check if we're already processing this file (circular #IncludeAgain)
    for file in includeChain {
        if (file = normalizedInput) {
            ; Build a nice error message showing the circular chain
            chainStr := ""
            for chainFile in includeChain {
                chainStr .= "-> " chainFile "`n"
            }
            chainStr .= "-> " normalizedInput " (Cycle)"
            
            Log.Fatal(Format(
                "#IncludeAgain cycle detected:`n{1}",
                chainStr
            ))
            ExitApp(1)
        }
    }
    
    ; Add current file to the chain
    includeChain.Push(normalizedInput)

    inBlockComment := false
    inIgnoreBlock := false

    loop read input {
        line := Trim(A_LoopReadLine, " `t`r`n")
        
        ; Check for ignore blocks
        if(inIgnoreBlock) {
            if(line.StartsWith(";@ahk2exe-ignoreend") || line.StartsWith(";@ahkbuild-ignoreend")) {
                Log.Trace(Format("Ignore block ending at line {1}: '{2}'", A_Index, A_LoopReadLine))
                inIgnoreBlock := false
            }
            continue
        }
        else if(line.StartsWith(";@ahk2exe-ignorebegin") || line.StartsWith(";@ahkbuild-ignorebegin")) {
            Log.Trace(Format("Ignore block beginning at line {1}: '{2}'", A_Index, A_LoopReadLine))
            inIgnoreBlock := true
            continue
        }

        ; Check for block comments
        if(inBlockComment) {
            if(line.EndsWith("*/")) {
                Log.Trace(Format("Block comment ending at line {1}: '{2}'", A_Index, A_LoopReadLine))
                inBlockComment := false
            }
            continue
        }
        else if(!args.keepComments && line.StartsWith("/*")) {
            Log.Trace(Format("Block comment beginning at line {1}: '{2}'", A_Index, A_LoopReadLine))

            ; Handle single-line block comments
            if(!line.EndsWith("*/"))
                inBlockComment := true
            continue
        }

        ; Check for regular comments, but keep directive-like comments
        if(!args.keepComments && line.StartsWith(";") && !line.StartsWith(";@")) {
            ; Comment but not directive (;@ahk2exe, etc)
            Log.Trace(Format("Ignoring comment at line {1}: '{2}'", A_Index, A_LoopReadLine))
            continue
        }

        ; Check for empty lines
        if (line == "" && !args.keepEmptyLines) {
            continue
        }
        
        ; Only include #Requires directives from the root script
        if (!isRoot && line.StartsWith("#Requires")) {
            continue
        }

        ; Include statement?
        if(RegExMatch(line, includePattern, &match)) {
            Log.Info(Format("Processing include directive at line {1}: '{2}'", A_Index, A_LoopReadLine))
            Log.Debug(Format("Match info: again='{1}' ignore='{2}' path='{3}'", match.again, match.ignore, match.path))

            filepath := ResolveInclude(match.path, input)
            filepath := StrReplace(filepath, "/", "\") ; Normalize filepath

            if(filepath == "" && match.ignore != "") {
                ; *i was set, so we'll continue
                Log.Warn(Format(
                    "Failed to resolve '{1}' at line {2} of '{3}', continuing because *i flag is set",
                    A_LoopReadLine, A_Index, input
                ))
                continue
            }
            else if(filepath == "") {
                ; Fatal - could not resolve path - abort immediately
                Log.Fatal(Format(
                    "Failed to resolve '{1}' at line {2} of '{3}'",
                    A_LoopReadLine, A_Index, input
                ))
                ExitApp(1)
            }

            Log.Info(Format("Resolved '{1}' statement to '{2}'", match.path, filepath))

            ; Skip if we've already included this and this isn't an #IncludeAgain directive
            if(match.again == "" && includeMap.Has(filepath)) {
                Log.Info(Format("File '{1}' has already been included, skipping", filepath))
                continue
            }

            includeMap[filepath] := true

            ; Add included file's contents, skip the actual line
            ParseInclude(filepath, output, includeMap, false, includeChain)
            continue
        }

        finalLine := A_LoopReadLine
        if(args.bitness != "any") {
            finalLine := StrReplace(finalLine, "A_PtrSize", args.bitness == "32" ? "4" : "8")
        }
        if(args.compiled) {
            finalLine := StrReplace(finalLine, "A_IsCompiled", "true")
        }
        if(args.dedent) {
            finalLine := LTrim(finalLine)
        }
        if(!args.keepComments) {
            finalLine := TrimTrailingComment(finalLine)
        }

        if(!args.dryrun) {
            output.WriteLine(finalLine)
        }
    }

    if(inIgnoreBlock) {
        Log.Warn(Format("Unclosed ignore block in '{1}'", input))
    }

    Log.Info(Format("Finished processing '{1}'", input))
    includeChain.Pop()
}

/**
 * Trims trailing comments off of a code line
 * @param {String} line the line
 * @returns {String} the trimmed line 
 */
TrimTrailingComment(line) {
    pos := 1
    while (pos := InStr(line, ";", , pos)) {
        if (pos == 1 || IsSpace(SubStr(line, pos - 1, 1))) {
            return RTrim(SubStr(line, 1, pos - 1))
        }
        pos++
    }
    return line
}

ResolveInclude(statement, currentFile) {
    ; Resolve %A_*% variables
    path := ResolveAVars(statement)
    if(path != statement)
        Log.Info(Format("Expanded '{1}' to '{2}'", statement, path))

    ; Relative / absolute path
    SplitPath(currentFile, , &fileDir := "")
    Log.Trace(Format("Resolving '{1}' relative to '{2}'", path, fileDir))

    ; Library?
    ; !IMPORTANT - follow documented behavior here: https://www.autohotkey.com/docs/v2/Scripts.htm#lib
    if(RegExMatch(path, "<(?'path'.+)>", &match)) {
        Log.Trace(Format("Searching library folders for {1}", match.path))
        
        SplitPath(A_AhkPath, , &ahkDir := "")
        SplitPath(args.input, , &mainFileDir := "")

        ; Order is important - local -> user -> standard
        searchPaths := [
            mainFileDir "\Lib\" match.path ".ahk",                  ; Local lib
            A_MyDocuments "\AutoHotkey\Lib\" match.path ".ahk",     ; User lib
            ahkDir "\Lib\" match.path ".ahk"                        
        ]

        for(candidate in searchPaths) {
            Log.Trace(Format("Checking for '{1}'...", candidate))
            if(FileExist(candidate)) {
                Log.Info(Format("Resolved '{1}' to '{2}'", path, candidate))
                return candidate
            }
        }

        Log.Error(Format("Failed to find '{1}' in any library directory", path))
        return ""
    }
    else {

        ; Setting working directory lets AHK do the path resolution work for us
        oldDir := A_WorkingDir
        A_WorkingDir := fileDir

        if(!FileExist(path)) {
            Log.Warn(Format("Path '{1}' does not exist relative to '{2}'", path, A_WorkingDir))
            A_WorkingDir := oldDir
            return ""
        }
        else {
            absolutePath := GetFullPathName(path)

            A_WorkingDir := oldDir
            return absolutePath
        }

    }

    return ""
}

/**
 * Parses command line arguments, returns the results
 * @returns {Object} parsed args
 */
ParseCommandLine() {
    /**
     * @type {ArgumentParser}
     */
    parser := ArgumentParser({description: "Parses include statements as a preprocessing step for AHK script compilation"})
    parser.AddPositional("input", {
        help: "The input file to process",
        validator: (val) => (FileExist(path := GetFullPathName(val)) ? 
            StrReplace(path, "/", "\") : 
            Error.Throw("Input file must exist: " path)
        ),
    })
    parser.AddPositional("output", { 
        help: "The path to the output file",
        validator: (val) => PathIsWriteableDirectory(val)
    })

    parser.AddOption("logLevel", { 
        long: "log", 
        choices: ["ALL", "TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"],
        envVar: "AHK_LOG_LEVEL",
        default: "INFO",
        help: "Log verbosity level"
    })

    parser.AddOption("loglocation", {
        long: "log-file",
        envVar: "AHK_LOG_FILE",
        default: A_ScriptDir "\preprocess-" A_Now ".log",
        help: "The log file path",
        validator: (val) => PathIsWriteableDirectory(val)
    })

    parser.AddOption("bitness", {
        long: "bitness",
        short: "b",
        default: "any",
        choices: ["any", "32", "64"],
        help: "If not 'any', replace A_PtrSize with literal pointer size"
    })

    parser.AddFlag("compiled", {
        long: "compiled",
        short: "c",
        help: "If present, replace A_IsCompiled with true"
    })
    parser.AddFlag("dedent", {
        long: "dedent",
        short: "d",
        help: "Dedent lines in the output file"
    })

    parser.AddFlag("dryrun", {
        long: "dry-run",
        help: "Perform a dry run; does not write to the destination file"
    })
    parser.AddFlag("keepEmptyLines", {
        long: "keep-empty",
        help: "Don't skip empty lines"
    })
    parser.AddFlag("keepComments", {
        long: "keep-comments",
        help: "Don't skip comments"
    })
    parser.AddFlag("overwrite", {
        long: "overwrite",
        short: "o",
        help: "Overwrite the output file if it exists"
    })

    return parser.Parse()
}