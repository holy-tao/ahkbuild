#Requires AutoHotkey v2.0

#Include buildTester.ahk
#Include YUnit\Assert.ahk

class MemberPruningTests {

    class BasicPruning {

        UnreferencedMethod_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    Used() => 1
                    Unused() => 0
                }

                obj := MyClass()
                obj.Used()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Used() => 1")
            Assert.NotInStr(shaken, "Unused")
        }

        ReferencedMethod_Survives() {
            code := "
            ( comments
                class MyClass {
                    Foo() => "foo"
                    Bar() => "bar"
                }

                obj := MyClass()
                obj.Foo()
                obj.Bar()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Foo() => `"foo`"")
            Assert.InStr(shaken, "Bar() => `"bar`"")
        }

        UnreferencedProperty_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    UsedProp => "used"
                    UnusedProp => "unused"
                }

                obj := MyClass()
                MsgBox(obj.UsedProp)
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "UsedProp")
            Assert.NotInStr(shaken, "UnusedProp")
        }

        UnreferencedStaticMethod_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    static UsedStatic() => 1
                    static UnusedStatic() => 0
                }

                MyClass.UsedStatic()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "UsedStatic")
            Assert.NotInStr(shaken, "UnusedStatic")
        }

        UnreferencedStaticProperty_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    static UsedStatic => 1
                    static UnusedStatic => 0
                }

                MyClass.UsedStatic()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "UsedStatic")
            Assert.NotInStr(shaken, "UnusedStatic")
        }

        MultipleClasses_PrunedIndependently() {
            code := "
            ( comments
                class A {
                    Foo() => 1
                    Bar() => 2
                }

                class B {
                    Foo() => 3
                    Baz() => 4
                }

                a := A()
                a.Foo()
                b := B()
                b.Baz()
            )"

            ; .Foo is referenced somewhere, so both A.Foo and B.Foo survive
            ; .Baz is referenced, so B.Baz survives
            ; .Bar is never referenced, so A.Bar is pruned
            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "Bar")
            Assert.InStr(shaken, "Baz")
        }

        TransitiveReference_KeepsMember() {
            code := "
            ( comments
                class MyClass {
                    Helper() => "help"
                    Caller() {
                        return this.Helper()
                    }
                }

                obj := MyClass()
                obj.Caller()
            )"

            ; .Caller is referenced directly, .Helper is referenced inside Caller
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Caller()")
            Assert.InStr(shaken, "Helper()")
        }
    }

    class MetaFunctions {

        MetaNew_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __New() {
                    }
                    Unused() => 0
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "__New()")
            Assert.NotInStr(shaken, "Unused")
        }

        MetaDelete_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __Delete() {
                    }
                    Unused() => 0
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "__Delete()")
            Assert.NotInStr(shaken, "Unused")
        }

        MetaCall_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __Call(name, params) {
                    }
                    Unused() => 0
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "__Call(")
            Assert.NotInStr(shaken, "Unused")
        }

        MetaGet_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __Get(name, params) {
                    }
                    Unused() => 0
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "__Get(")
            Assert.NotInStr(shaken, "Unused")
        }

        MetaSet_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __Set(name, params, value) {
                    }
                    Unused() => 0
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "__Set(")
            Assert.NotInStr(shaken, "Unused")
        }

        Call_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    static Call() {
                    }
                    Unused() => 0
                }

                MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "static Call()")
            Assert.NotInStr(shaken, "Unused")
        }
    }

    class DynamicAccess {

        FullyDynamic_DisablesPruning() {
            code := "
            ( comments
                class MyClass {
                    NeverCalled() => 0
                }

                obj := MyClass()
                name := "anything"
                obj.%name%()
            )"

            ; Fully dynamic access → member pruning disabled → NeverCalled kept
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "NeverCalled")
        }

        DynamicWithPrefix_KeepsMatchingMembers() {
            code := "
            ( comments
                class MyClass {
                    GetName() => "name"
                    GetAge() => 42
                    SetValue(v) {
                    }
                }

                obj := MyClass()
                prop := "Name"
                obj.Get%prop%()
            )"

            ; Prefix "Get" - GetName and GetAge survive, SetValue pruned
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "GetName")
            Assert.InStr(shaken, "GetAge")
            Assert.NotInStr(shaken, "SetValue")
        }

        DynamicWithLiteralPrefix_KeepsMatchingMembers() {
            code := "
            ( comments
                class MyClass {
                    GetName() => "name"
                    GetAge() => 42
                    SetValue(v) {
                    }
                }

                obj := MyClass()
                prop := "Name"
                obj.%"Get" prop%()
            )"

            ; Prefix "Get" - GetName and GetAge survive, SetValue pruned
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "GetName")
            Assert.InStr(shaken, "GetAge")
            Assert.NotInStr(shaken, "SetValue")
        }

        DynamicWithSuffix_KeepsMatchingMembers() {
            code := "
            ( comments
                class MyClass {
                    NameHandler() => 1
                    AgeHandler() => 2
                    Process() => 3
                }

                obj := MyClass()
                kind := "Name"
                obj.%kind%Handler()
            )"

            ; Suffix "Handler" - NameHandler and AgeHandler survive, Process pruned
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "NameHandler")
            Assert.InStr(shaken, "AgeHandler")
            Assert.NotInStr(shaken, "Process")
        }

        DynamicWithLiteralSuffix_KeepsMatchingMembers() {
            code := "
            ( comments
                class MyClass {
                    NameHandler() => 1
                    AgeHandler() => 2
                    Process() => 3
                }

                obj := MyClass()
                kind := "Name"
                obj.%kind . "Handler"%()
            )"

            ; Suffix "Handler" - NameHandler and AgeHandler survive, Process pruned
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "NameHandler")
            Assert.InStr(shaken, "AgeHandler")
            Assert.NotInStr(shaken, "Process")
        }

        DynamicStringLiteral_AddsExactName() {
            code := "
            ( comments
                class MyClass {
                    Target() => 1
                    Other() => 0
                }

                obj := MyClass()
                obj.%"Target"%()
            )"

            ; Inner string literal → exact name "Target"
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Target")
            Assert.NotInStr(shaken, "Other")
        }
    }

    class ReflectionFunctions {

        ObjBindMethod_LiteralString_AddsName() {
            code := "
            ( comments
                class MyClass {
                    Bound() => "bound"
                    Unbound() => "unbound"
                }

                obj := MyClass()
                fn := ObjBindMethod(obj, "Bound")
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "Bound")
            Assert.NotInStr(shaken, "Unbound")
        }

        ObjBindMethod_NonLiteral_DisablesPruning() {
            code := "
            ( comments
                class MyClass {
                    NeverCalled() => 0
                }

                obj := MyClass()
                name := "NeverCalled"
                fn := ObjBindMethod(obj, name)
            )"

            ; Non-literal method name → member pruning disabled
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "NeverCalled")
        }

        ObjBindMethod_ConcatString_AddsPrefix() {
            code := "
            ( comments
                class MyClass {
                    GetName() => "name"
                    GetAge() => 42
                    SetValue(v) {
                    }
                }

                obj := MyClass()
                prop := "Name"
                fn := ObjBindMethod(obj, "Get" . prop)
            )"

            ; Concat with literal prefix "Get" → GetName and GetAge survive
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "GetName")
            Assert.InStr(shaken, "GetAge")
            Assert.NotInStr(shaken, "SetValue")
        }
    }

    class DeadClassUnchanged {

        DeadClass_StillPruned() {
            code := "
            ( comments
                class DeadClass {
                    Method() => 1
                }

                MsgBox("no class usage")
            )"

            ; Dead class is still removed entirely
            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "DeadClass")
            Assert.NotInStr(shaken, "Method")
        }

        LiveClass_WithAllMembersReferenced_FullyKept() {
            code := "
            ( comments
                class MyClass {
                    A() => 1
                    B() => 2
                }

                obj := MyClass()
                obj.A()
                obj.B()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "A() => 1")
            Assert.InStr(shaken, "B() => 2")
        }
    }

    class DefineProp {

        UnreferencedProp_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    __New() {
                        this.DefineProp("Unused", {Get: (*) => 0})
                    }
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "Unused")
            Assert.NotInStr(shaken, "DefineProp")
        }

        ReferencedProp_Survives() {
            code := "
            ( comments
                class MyClass {
                    __New() {
                        this.DefineProp("Used", {Get: (*) => 1})
                    }
                }

                obj := MyClass()
                MsgBox(obj.Used)
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "DefineProp")
            Assert.InStr(shaken, "Used")
        }

        NonLiteralName_KeptConservatively() {
            code := "
            ( comments
                class MyClass {
                    __New() {
                        name := "Dynamic"
                        this.DefineProp(name, {Get: (*) => 1})
                    }
                }

                obj := MyClass()
            )"

            ; Non-literal property name — cannot determine statically, keep the call
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "DefineProp")
        }

        ProtectedName_NeverPruned() {
            code := "
            ( comments
                class MyClass {
                    __New() {
                        this.DefineProp("__Get", {Call: (this, name, params) => ""})
                    }
                }

                obj := MyClass()
            )"

            ; Protected meta-function name — never pruned
            shaken := BuildTester.TreeShake(code)
            Assert.InStr(shaken, "DefineProp")
            Assert.InStr(shaken, "__Get")
        }

        TransitivePruning() {
            code := "
            ( comments
                MyHelper(this) {
                    return 42
                }

                class MyClass {
                    __New() {
                        this.DefineProp("Unused", {Get: MyHelper})
                    }
                }

                obj := MyClass()
            )"

            ; MyHelper is only referenced from the pruned DefineProp descriptor,
            ; so it should also be dead
            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "Unused")
            Assert.NotInStr(shaken, "MyHelper")
        }

        PrototypeForm_IsPruned() {
            code := "
            ( comments
                class MyClass {
                    static __New() {
                        this.Prototype.DefineProp("Unused", {Get: (*) => 0})
                    }
                }

                obj := MyClass()
            )"

            shaken := BuildTester.TreeShake(code)
            Assert.NotInStr(shaken, "Unused")
            Assert.NotInStr(shaken, "DefineProp")
        }
    }
}
