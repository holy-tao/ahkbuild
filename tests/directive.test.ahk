#Requires AutoHotkey v2.0

#Include buildTester.ahk
#Include YUnit\Assert.ahk

class DirectiveTests {

    class Parsing {

        Directive_BeforeFunction_DoesNotCreateOpaqueNode() {
            code := "
            (
                ;@TestDirective
                Referenced() => 1
                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            ; The directive should be preserved as-is (it's before a live node)
            Assert.InStr(shaken, ";@TestDirective")
            Assert.InStr(shaken, "Referenced() => 1")
        }

        Directive_WithArguments_PreservedInOutput() {
            code := "
            (
                ;@AhkBuild-SafeDynamicRef SomeArg
                Referenced() => 1
                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@AhkBuild-SafeDynamicRef SomeArg")
            Assert.InStr(shaken, "Referenced() => 1")
        }

        MultipleDirectives_BeforeOneNode_AllPreserved() {
            code := "
            (
                ;@DirectiveOne
                ;@DirectiveTwo WithArgs
                Referenced() => 1
                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@DirectiveOne")
            Assert.InStr(shaken, ";@DirectiveTwo WithArgs")
            Assert.InStr(shaken, "Referenced() => 1")
        }
    }

    class TreeShakeCleanup {

        Directive_OnDeadFunction_IsRemoved() {
            code := "
            (
                ;@TestDirective
                Unreferenced() => 0
                MsgBox("Test")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, ";@TestDirective")
            Assert.NotInStr(shaken, "Unreferenced")
        }

        Directive_OnLiveFunction_Survives() {
            code := "
            (
                ;@TestDirective
                Referenced() => 1
                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@TestDirective")
            Assert.InStr(shaken, "Referenced() => 1")
        }

        MultipleDirectives_OnDeadFunction_AllRemoved() {
            code := "
            (
                ;@DirectiveOne
                ;@DirectiveTwo
                ;@DirectiveThree
                Unreferenced() => 0
                MsgBox("Test")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, ";@DirectiveOne")
            Assert.NotInStr(shaken, ";@DirectiveTwo")
            Assert.NotInStr(shaken, ";@DirectiveThree")
            Assert.NotInStr(shaken, "Unreferenced")
        }

        Directive_OnDeadFunction_DoesNotAffectAdjacentLiveCode() {
            code := "
            (
                ;@KeepMe
                Referenced() => 1

                ;@RemoveMe
                Unreferenced() => 0

                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@KeepMe")
            Assert.InStr(shaken, "Referenced() => 1")
            Assert.NotInStr(shaken, ";@RemoveMe")
            Assert.NotInStr(shaken, "Unreferenced")
        }

        Directive_OnDeadClass_IsRemoved() {
            code := "
            (
                ;@SomeDirective
                class DeadClass {
                    Method() => 1
                }

                MsgBox("alive")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, ";@SomeDirective")
            Assert.NotInStr(shaken, "DeadClass")
        }
    }

    class ClassBody {

        Directive_BeforeMethod_InClassBody_Survives() {
            code := "
            (
                class MyClass {
                    ;@TestDirective
                    Used() => 1
                }

                obj := MyClass()
                obj.Used()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@TestDirective")
            Assert.InStr(shaken, "Used() => 1")
        }

        Directive_BeforeDeadMethod_InClassBody_IsRemoved() {
            code := "
            (
                class MyClass {
                    ;@TestDirective
                    Unused() => 0

                    Used() => 1
                }

                obj := MyClass()
                obj.Used()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, ";@TestDirective")
            Assert.NotInStr(shaken, "Unused")
            Assert.InStr(shaken, "Used() => 1")
        }
    }

    class BlockLevel {

        Directive_InsideBlock_PreservedWithLiveCode() {
            code := "
            (
                Outer() {
                    ;@BlockDirective
                    Inner() => 1
                    return Inner()
                }

                Outer()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, ";@BlockDirective")
            Assert.InStr(shaken, "Inner() => 1")
        }

        Directive_InsideBlock_RemovedWithDeadCode() {
            code := "
            (
                Outer() {
                    ;@BlockDirective
                    Dead() => 0
                    Inner() => 1
                    return Inner()
                }

                Outer()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, ";@BlockDirective")
            Assert.NotInStr(shaken, "Dead")
            Assert.InStr(shaken, "Inner() => 1")
        }
    }
}
