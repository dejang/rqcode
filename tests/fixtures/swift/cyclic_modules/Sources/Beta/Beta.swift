import Alpha

public func betaStep(_ value: Int) -> Int {
    return value > 0 ? alphaStep(value - 1) : 1
}
