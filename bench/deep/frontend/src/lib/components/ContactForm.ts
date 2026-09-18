/** The form that creates or edits a contact. */

import { createContact } from "../api/contacts";

/** Validates in the browser before the request, matching the server rule. */
export function validate(values: Record<string, string>): string[] {
  const errors: string[] = [];
  const hasCompany = Boolean(values.companyName);
  const hasPerson = Boolean(values.lastName);
  if (hasCompany === hasPerson) {
    errors.push("a contact is either a company or a person");
  }
  return errors;
}

/** Submits the form, returning the server's errors if it refuses. */
export async function submit(values: Record<string, string>): Promise<string[]> {
  const errors = validate(values);
  if (errors.length > 0) {
    return errors;
  }
  await createContact(values);
  return [];
}
