#Requires AutoHotkey v2.0

#Include buildTester.ahk
#Include YUnit\Assert.ahk

class TreeShakeTests {

    class Functions {

        Function_Unreferenced_DoesNotSurvive() {
            code := "
            ( comments
                Unreferenced() => MsgBox("Hello, World!")
                MsgBox("Test")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "Unreferenced()")
        }

        Function_Referenced_Survives() {
            code := "
            ( comments
                Referenced() => MsgBox("Hello, World!")
                Referenced()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Referenced() => MsgBox(`"Hello, World!`")")
        }

        Function_TransitivelyReferenced_Survives() {
            code := "
            ( comments
                Transitive() => "Hello"
                Outer() {
                    return Transitive() "World"
                }
                
                MsgBox(Outer())
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Transitive() => `"Hello`"")
            Assert.InStr(shaken, "Outer() ")
        }

        Function_ReferencedInReturnStatements_Survives() {
            code := "
            ( comments
                Transitive() => "Hello"
                Outer() {
                    return Transitive() "World"
                }
                
                MsgBox(Outer())
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Transitive() => `"Hello`"")
            Assert.InStr(shaken, "Outer() ")
        }

        InnerFunctions_Referenced_AreShakenAppropriately() {
            code := "
            ( comments
                Outer() {
                    ReferencedInner() => 1
                    UnreferencedInner() => 0

                    return ReferencedInner()
                }
                
                MsgBox(Outer())
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "ReferencedInner() => 1")
            Assert.NotInStr(shaken, "UnreferencedInner()")
        }

        Function_ReferencedAsIdentifier_Survives() {
            code := "
            ( comments
                Referenced() => MsgBox("Hello, World!")
                return Referenced
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Referenced() => MsgBox(`"Hello, World!`")")
        }

        Function_ReferencedInExpression_Survives() {
            code := "
            ( comments
                Referenced(arg) => arg * 2
                return Referenced.Bind(4)
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Referenced(arg) => arg * 2")
        }

        Function_ReferencedInIfStatement_Survives() {
            code := "
            ( comments
                Referenced() => true
                if Referenced() {
                    MsgBox("true")
                }
                else {
                    MsgBox("false")    
                }
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Referenced() => true")
        }

        Function_ReferencedAsIdentifierInTernaryExpr_Survies() {
            code := "
            ( comments
                Outer(val) {
                    Inner1() => "one"
                    Inner2() => "two"

                    return val ? Inner1 : Inner2
                }

                Outer(true)
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Inner1() => `"one`"")
            Assert.InStr(shaken, "Inner2() => `"two`"")
        }

        Function_ReferencedAsParamDefault_Survies() {
            code := "
            ( comments
                Referenced() => "hello"

                Referencer(arg1, arg2 := Referenced()) {
                    return arg1 . arg2
                }

                Referencer("world")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Referenced() => `"hello`"")
            Assert.InStr(shaken, "Referencer(arg1, arg2 := Referenced())")
        }
    }

    class MemberAccess {
        MemberAccess_ResolvedThroughChain() {
            code := "
            ( comments
                class Outer {
                    ; With bug, outer is improperly pruned
                    class Inner  {
                        __New(params*) => MsgBox(params.length)
                    }
                }

                class Second {
                    Fn() {
                        Outer.Inner(1, 2)
                    }
                }

                var := Second()
                var.Fn()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "class Outer")
            Assert.InStr(shaken, "class Inner")
            Assert.InStr(shaken, "class Second")
            Assert.InStr(shaken, "Fn() {")
            Assert.InStr(shaken, "__New(params*)")
        }
    }
}