#Include <util\text\RegEx>

/**
 * Utilities for directives
 */
class Directives {
    /**
     * Parses the arguments to an ;@AhkBuild-ResolvesTo directive.
     * @param {String} args Space delimited list of arguments, optionally quoted
     * @returns {Array<String>} array of parsed arguments
     */
    static ParseResolvesToArgs(args) {
        static pattern := "`"[^`"]*`"|'[^']*'|\w+|\\S+"
        return RegEx.Matches(args, pattern)
    }
}