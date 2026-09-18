/** Typed calls against the contact routes. */

import { request } from "./client";

/** Registers a contact and returns what the server stored. */
export async function createContact(body: unknown): Promise<unknown> {
  return request("/contacts", { method: "POST", body: JSON.stringify(body) });
}

/** One contact by id. */
export async function getContact(id: string): Promise<unknown> {
  return request(`/contacts/${id}`);
}

/** A page of contacts. */
export async function listContacts(cursor?: string): Promise<unknown> {
  return request(cursor ? `/contacts?cursor=${cursor}` : "/contacts");
}
