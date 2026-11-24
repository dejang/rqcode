test("invoice total with cap", () => {
    const items = [10, -5, 40, 25, 30];
    let acc = 0;
    for (const item of items) {
        if (item > 0) {
            acc += item;
        }
        if (acc > 80) {
            acc = 80;
        }
    }
    const finalTotal = acc > 15 ? acc - 15 : 0;
    expect(finalTotal).toBe(65);
});

test("cart subtotal with ceiling", () => {
    const prices = [12, -3, 35, 20, 45];
    let sum = 0;
    for (const price of prices) {
        if (price > 0) {
            sum += price;
        }
        if (sum > 90) {
            sum = 90;
        }
    }
    const payable = sum > 10 ? sum - 10 : 0;
    expect(payable).toBe(80);
});

test("distinct string normalization", () => {
    const raw = "  alpha, beta, gamma  ";
    const parts = raw.trim().split(",").map((p) => p.trim());
    expect(parts.length).toBe(3);
    expect(parts[0]).toBe("alpha");
});
