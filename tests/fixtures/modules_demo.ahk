#Import X {Calculate as CalculateX}
#Import Y {*}

MyVar := 1
MsgBox Calculate()
MsgBox CalculateX()
MsgBox Check(3)

Calculate() => 1

#Module X
export Calculate() {
    return 2
}

#Module Y
export Calculate() {
    return 3
}
export Check(n) {
    return n = Calculate()
}
