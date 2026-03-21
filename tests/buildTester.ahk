#Requires AutoHotkey v2.0

#Include <util\shell\Cmd>

class BuildTester {

    /**
     * Runs the tree-shaking algorithm over some code stored in a variable
     * @param {String} filecontents the code to tree-shake
     */
    static TreeShake(filecontents) {
        timestamp := A_Now . A_MSec

        if(FileExist(tmpFile := A_Temp "\buildTester-" timestamp ".tmp.ahk"))
            FileDelete(tmpFile)

        if(FileExist(outFile := A_Temp "\buildTester-" timestamp ".out.ahk"))
            FileDelete(outFile)

        if(FileExist(logFile := A_Temp "\buildTester-" timestamp ".log"))
            FileDelete(logFile)
        
        FileAppend(filecontents, tmpFile)

        buildScriptPath := CmdExpect("git rev-parse --show-toplevel") "\build.ahk"
        cmd := Format('"{1}" "{2}" "{3}" "{4}" --tree-shake --overwrite --log=TRACE --log-file={5}',
            A_AhkPath, buildScriptPath, tmpFile, outFile, logFile)

        output := CmdExpect(cmd)
        FileAppend(output, "*")

        ; It's fine to throw here on failure, we're in a test method
        shaken := FileRead(outFile)
        FileDelete(tmpFile)
        FileDelete(outFile)
        FileDelete(logFile)

        return shaken
    }
}