; Group B (shared by P-B and P-C): structurally identical to GroupA, also #Module Helper.
#Import Helper
export GetVal() {
    return Helper.Val()
}

#Module Helper
export Val() {
    return "B"
}
