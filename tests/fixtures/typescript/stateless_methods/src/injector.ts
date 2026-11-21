export class TextInjector {
    private delayMs: number;
    private clipboardSnapshot: string;

    constructor(delayMs: number, clipboardSnapshot: string) {
        this.delayMs = delayMs;
        this.clipboardSnapshot = clipboardSnapshot;
    }

    static create(): TextInjector {
        return new TextInjector(50, "");
    }

    public inject(text: string): string {
        this.clipboardSnapshot = text;
        return `${this.delayMs}:${this.clipboardSnapshot}`;
    }

    public getDelayMs(): number {
        return this.delayMs;
    }

    static normalizeLineEndings(raw: string): string {
        return raw.replace(/\r\n/g, "\n");
    }

    static isTrustedProcess(pid: number): boolean {
        return pid > 0;
    }
}
