/************************************************************************
 * @description Functionality for loading a file and parsing it into a tree-sitter AST
 * @author 
 * @date 2026/02/26
 * @version 0.0.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include <log4ahk\Log>
#Include <Extensions\Errors\All>

#Include <tree-sitter\TSLanguage>
#Include <tree-sitter\TSParser>
#Include <tree-sitter\TSTree>
#Include <tree-sitter\TSNode>
#Include <tree-sitter\TSQuery>
#Include <tree-sitter\TSQueryCursor>

#DllLoad ../bin/tree-sitter.dll
#DllLoad ../bin/tree-sitter-autohotkey.dll

/**
 * Loads a file into memory and builds a tree-sitter AST. If there are errors, displays those errors
 * and quits immediately.
 * 
 * @param {String} filepath the path to process
 * @returns {TSTree} the parsed syntax tree
 */
BuildAST(filepath) {
    ; TODO investigate memory mapping inputs
    Log.Info("Building AST for '" filepath "'")
    source := FileRead(filepath, "RAW")
    Log.Debug(Format("Source buffer: {1} bytes @ 0x{2:X}", source.size, source.ptr))

    lang := TSLanguage(DllCall("tree-sitter-autohotkey\tree_sitter_autohotkey", "cdecl ptr"))
    Log.Debug(Format("Loaded tree-sitter language '{1}' v{2} with ABI version {3}", 
        lang.name, lang.LanguageVersion, lang.AbiVersion))

    parser := TSParser(lang)

    Log.Debug("Parsing source file")
    ast := parser.Parse(source)

    if(ast.root.HasError) {
        DisplayParseError(ast, filepath)
        Exit(1)
    }

    WarnForIncludes(ast)

    Log.Info("Parsing complete")

    return ast
}

/**
 * Check to see if the ast contains any `#Include` directives and warn if it does
 * @param {TSTree} ast the tree to check 
 */
WarnForIncludes(ast) {
    queryCursor := ast.Query("(include_directive) @incl (include_again_directive) @incl")
    while match := queryCursor.NextMatch() {
        for capture in match.captures {
            msg := "#Include directive will not be resolved. Did you forget to run the preprocessor?`r`n" capture.node.Text
            Log.Warn(msg)
        }
    }
}

/**
 * Prints a nicely formatted error message to the console / any logs for a tree-sitter parsing error.
 * 
 * @param {TSTree} ast the tree with the error node(s) 
 * @param {String} filepath path to the file that errored 
 */
DisplayParseError(ast, filepath) {
    static ctxLines := 2
    Log.Fatal("Encountered error(s) parsing '" filepath "':")

    f := FileOpen(filepath, "r")

    /** @type {TSQueryCursor}*/
    errorNodes := ast.Query("(ERROR) @err")

    while(err := errorNodes.NextCapture()) {
        start := err.match.captures[err.captureIndex + 1].node.StartPoint
        end := err.match.captures[err.captureIndex + 1].node.EndPoint

        context := ""

        f.Pos := 0

        ; Skip to error location - lines
        fileLine := 0
        Loop(Max(start.row - ctxLines, 0)){
            f.ReadLine()
            fileLine++
        }

        padWidth := StrLen(String(fileLine + (2 * ctxLines) + 1))
        fmtString := "{1:" padWidth "} {2} | {3}`n"

        ; Read lines above where error was thrown
        Loop(ctxLines) {
            context .= Format(fmtString, ++fileLine, " ", f.ReadLine())
        }

        logLine := f.ReadLine()
        underline := ""
        loop(start.column) {
            underline .= " "
        }

        ; Build the underline
        loop(end.row == start.row ? (end.column - start.column) : (StrLen(logLine) - start.column))
            underline .= "~"

        context .= Format(fmtString, ++fileLine, ">", logLine)
        context .= Format(fmtString, " ", " ", underline)

        ; Read lines below thrower location
        Loop(ctxLines) {
            if(f.AtEOF) {
                context .= Format("{1:-" padWidth "}   |`n", "EOF")
                break
            }
            context .= Format(fmtString, ++fileLine, " ", f.ReadLine())
        }

        msg := "Parse error (zero-indexed) from " String(start) " - " String(end) ":`r`n" context

        Log.Fatal(msg)
    }

    Log.Fatal("Exiting")
}