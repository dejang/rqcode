import Beta

public func alphaStep(_ value: Int) -> Int {
    return value > 0 ? betaStep(value - 1) : 0
}
