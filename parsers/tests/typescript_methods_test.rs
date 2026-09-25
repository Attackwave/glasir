//! A TypeScript method with a return type annotation was no definition: the
//! scan after the parameters stopped at the type's `<`. Angular and NestJS
//! annotate nearly every method, so a service's calls fell to its class.

use native_parsers::Language;

#[test]
fn annotated_and_generic_methods_are_definitions() {
    let src = "export class S {\n  create(a: NewA): Observable<IA> {\n    return this.http.post<IA>(this.u, a);\n  }\n  pick<T>(a: T): Promise<T | undefined> {\n    return go(a);\n  }\n  many(): string[] {\n    return list();\n  }\n}\n";
    let f = native_parsers::rules::active().parse(Language::TypeScript, src);
    for name in ["S", "create", "pick", "many"] {
        assert!(f.defines.iter().any(|d| d == name), "{name} missing: {:?}", f.defines);
    }
    assert!(f.calls.iter().any(|(from, to, _)| from == "pick" && to == "go"), "{:?}", f.calls);
    assert!(f.calls.iter().any(|(from, to, _)| from == "many" && to == "list"), "{:?}", f.calls);
}
