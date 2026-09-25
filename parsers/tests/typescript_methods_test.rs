//! A TypeScript method with a return type annotation was no definition: the
//! scan after the parameters stopped at the type's `<`. Angular and NestJS
//! annotate nearly every method, so a service's calls fell to its class.

use native_parsers::Language;

#[test]
fn annotated_and_generic_methods_are_definitions() {
    let src = "export class S {\n  create(a: NewA): Observable<IA> {\n    return this.http.post<IA>(this.u, a);\n  }\n  pick<T>(a: T): Promise<T | undefined> {\n    return go(a);\n  }\n  many(): string[] {\n    return list();\n  }\n  delete(id: number): Observable<undefined> {\n    return remove(id);\n  }\n}\n";
    let f = native_parsers::rules::active().parse(Language::TypeScript, src);
    for name in ["S", "create", "pick", "many", "delete"] {
        assert!(f.defines.iter().any(|d| d == name), "{name} missing: {:?}", f.defines);
    }
    assert!(f.calls.iter().any(|(from, to, _)| from == "pick" && to == "go"), "{:?}", f.calls);
    assert!(f.calls.iter().any(|(from, to, _)| from == "many" && to == "list"), "{:?}", f.calls);
}

#[test]
fn a_brace_in_a_signature_is_a_type_not_a_scope() {
    let facts = native_parsers::rules::active().parse(
        Language::TypeScript,
        "export function f(input: A | B<{ id?: string }>): any {\n  init(input);\n}\n\nclass C {\n  m(o: { a: string }): { b: number } {\n    return make(o);\n  }\n}\n",
    );
    for (caller, callee) in [("f", "init"), ("m", "make")] {
        assert!(
            facts.calls.iter().any(|c| c.0 == caller && c.1 == callee),
            "{caller} -> {callee}: {:?}",
            facts.calls
        );
    }
}
