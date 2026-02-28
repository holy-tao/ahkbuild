#Requires AutoHotkey v2.0

#Include <log4ahk\Log>

/**
 * Resolves shortPath to a full path relative to the current working directory
 * @see https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfullpathnamew
 */
GetFullPathName(shortPath, bufLen := 260) {
    fullPath := "", VarSetStrCapacity(&fullPath, bufLen)

    actualLen := DllCall("GetFullPathNameW", 
        "str", shortPath,
        "int", bufLen,
        "str", fullPath,
        "ptr*", &lpFilePart := 0)

    Log.Debug(Format("GetFullPathNameW('{1}') returned {2}", shortPath, actualLen))
    Log.Debug(Format("fullPath is '{1}'", fullPath))

    if(actualLen == 0) {
        ; Error
        throw OSError(A_LastError, , Format("GetFullPathNameW('{1}')", shortPath))
    }
    else if(actualLen > bufLen) {
        ; Buffer too small
        return GetFullPathName(shortPath, actualLen + 1)
    }
    else {
        return fullPath
    }
}

/**
 * Resolves A_* variables in a string to their values and returns the string
 * @param {String} str the string to process
 * @returns {String} 
 */
ResolveAVars(str) {
    static varpattern := "s)%(?'varname'A_\w+)%"

    startPos := 1
    while(RegExMatch(str, varpattern, &match, startPos)) {
        try {
            val := %match.varname%
            Log.Trace(Format("Replacing '{1}' with '{2}' at position {3} of '{4}'", match[], val, match.Pos, str))
            str := SubStr(str, 1, match.Pos - 1) . val . SubStr(str, match.Pos + match.Len)
            startPos := match.Pos + StrLen(val)
        }
        catch Error as err {
            Log.Warn(Format("Failed to replace A_* var '{1}' at position {2} of '{3}': {4}", match.varname, match.pos, str, err.Message))
            Log.Debug(err)
            startPos += match.Len
        }
    }

    return str
}

/**
 * Expands a path and throws an Error if it's not in a writeable directory
 * @param {String} path the path
 * @returns {String} the expanded path, if valid 
 */
PathIsWriteableDirectory(path) {
    fullPath := GetFullPathName(path)
    SplitPath(fullPath, &filename, &directory)

    if(!DirExist(directory)) {
        throw Error(Format("Directory '{1}' does not exist", directory))
    }

    return fullPath
}