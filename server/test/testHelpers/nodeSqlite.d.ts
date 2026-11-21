// The server has no @types/node; this declares the slice of node:sqlite that
// realSqlExecutor.ts uses (it only runs in the plain-Node vitest project).
declare module "node:sqlite" {
  export class StatementSync {
    all(...params: unknown[]): Record<string, unknown>[];
  }
  export class DatabaseSync {
    constructor(path: string);
    exec(sql: string): void;
    prepare(sql: string): StatementSync;
    close(): void;
  }
}
