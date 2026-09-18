/**
 * Who is logged in, held in one store the whole app reads.
 *
 * The token lives in localStorage rather than in memory so a reload does not
 * log the user out; everything derived from it is recomputed on read.
 */

export const TOKEN_KEY = "token";

/** Stores the token and marks the session active. */
export function login(token: string): void {
  localStorage.setItem(TOKEN_KEY, token);
}

/** Clears the token. Every derived store empties with it. */
export function logout(): void {
  localStorage.removeItem(TOKEN_KEY);
}

/** True while a token is present. Does not check whether it still validates. */
export function isAuthenticated(): boolean {
  return localStorage.getItem(TOKEN_KEY) !== null;
}
