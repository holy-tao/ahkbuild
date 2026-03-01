#Requires AutoHotkey v2.0

#Include <Extensions\Errors\All>
#Include <argparse\ArgumentParser>
#Include <log4ahk\Log>
#Include <log4ahk\appenders\FileAppender>
#Include <tree-sitter\TSLanguage>
#Include <tree-sitter\TSParser>

#Include ../Common/utils.ahk
#Include astbuilder.ahk
#Include irbuilder.ahk
#Include treeshake.ahk
#Include emitter.ahk

#DllLoad ../bin/tree-sitter.dll
#DllLoad ../bin/tree-sitter-autohotkey.dll

Main(cmdLine := A_Args) {
    args := ParseCommandLine(cmdLine)

    FileAppend("", args.loglocation)

    Log.Configure(args.loglevel)
        .ToLogger(Log.Logger()
            .WithAppender(FileAppender(args.loglocation, 50))
            .WithAppender(ConsoleAppender().WithPattern("{Level}: {Message}"))
    )

    ; All uncaught errors are fatal
    OnError((thrown, mode) => (Log.Fatal(thrown), ExitApp(1)))

    if(FileExist(args.output) && !args.overwrite) {
        Log.Fatal(Format(
            "Output file '{1}' already exists.`r`nSpecify --overwrite or -o to overwrite it if this is intentional.",
            args.output
        ))
        ExitApp(1)
    }

    ast := BuildAST(args.input)

    ; Build IR from the AST
    builder := IRBuilder()
    program := builder.Build(ast, FileRead(args.input, "RAW"))

    Log.Info(Format("IR complete: {1} top-level nodes", program.body.Length))
    if Log.CurrentLevel <= Log.Level.TRACE {
        Log.Trace("Symbol table:`r`n" builder.symbolTable.TraceDump())
    }

    ; Tree-shaking pass
    if args.treeShake {
        shaker := TreeShaker()
        dead := shaker.Run(program)
        Log.Info(Format("Tree-shaking removed {1} dead symbols", dead.Length))
        if Log.CurrentLevel <= Log.Level.TRACE {
            Log.Trace("Symbol table after tree-shaking:`r`n" builder.symbolTable.TraceDump())
        }
    }

    Log.Info("Writing output to " args.output "...")
    writer := Emitter()
    outFile := FileOpen(args.output, "w", "UTF-8")
    outFile.Write(writer.Emit(program))
    outFile.Close()

    Log.Info("Done")
    ExitApp(0)
}

/**
 * Parse command line arguments
 * @returns {Object} object with parsed arguments
 */
ParseCommandLine(cmdLine) {
    /**
     * @type {ArgumentParser}
     */
    parser := ArgumentParser({description: "Builds AutoHotkey scripts for compilation"})

    parser.AddPositional("input", {
        type: "String",
        help: "The script to process",
        validator: (val) => (FileExist(path := GetFullPathName(val)) ? 
            StrReplace(path, "/", "\") : 
            Error.Throw("Input file must exist: " path)
        )
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
        default: A_ScriptDir "\ahkbuild-" A_Now ".log",
        help: "The log file path",
        validator: (val) => PathIsWriteableDirectory(val)
    })

    parser.AddFlag("overwrite", {
        long: "overwrite",
        short: "o",
        help: "Overwrite the output file if it exists"
    })

    parser.AddFlag("treeShake", {
        long: "tree-shake",
        help: "Enable tree-shaking (dead code elimination)"
    })

    return parser.Parse(cmdLine)
}

if(A_ScriptName == "build.ahk" || A_IsCompiled) {
    Main()
}