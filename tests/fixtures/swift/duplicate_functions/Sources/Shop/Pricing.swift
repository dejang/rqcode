public func computeRetailTotal(unitPrices: [Double], taxRate: Double, memberBonus: Double) -> Double {
    var runningTotal = 0.0
    for price in unitPrices {
        if price > 0.0 {
            runningTotal += price
        }
    }
    let afterTax = runningTotal + runningTotal * taxRate
    if afterTax > 100.0 {
        return afterTax - memberBonus
    }
    return afterTax
}

public func computeWholesaleTotal(itemCosts: [Double], levyRate: Double, volumeRebate: Double) -> Double {
    var accumulatedCost = 0.0
    for cost in itemCosts {
        if cost > 0.0 {
            accumulatedCost += cost
        }
    }
    let withLevy = accumulatedCost + accumulatedCost * levyRate
    if withLevy > 500.0 {
        return withLevy - volumeRebate
    }
    return withLevy
}
