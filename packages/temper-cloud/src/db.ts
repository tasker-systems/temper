import { type NeonQueryFunction, neon } from "@neondatabase/serverless";

export function getDb(): NeonQueryFunction<false, false> {
  const databaseUrl = process.env.DATABASE_URL;
  if (!databaseUrl) {
    throw new Error("DATABASE_URL environment variable is required");
  }
  return neon(databaseUrl);
}

export type NeonClient = NeonQueryFunction<false, false>;
