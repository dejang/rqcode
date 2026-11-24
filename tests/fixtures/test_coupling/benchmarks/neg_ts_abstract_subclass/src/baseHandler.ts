export abstract class BaseHandler {
  public execute(payload: string): string {
    return this.handle(payload.trim());
  }

  protected abstract handle(payload: string): string;
}
