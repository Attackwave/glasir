use native_parsers::Language;

#[test]
fn test_category_01_backend() {
    // 1. Rust with attributes and regular comments
    let rs_code = r#"
// Calculates the total with tax
// Handles currency conversion
#[derive(Debug, Clone)]
#[inline]
pub async fn calculate_total(price: f64) -> f64 {
    let sub = compute_subtotal(price);
    db.save(sub);
    sub
}

/**
 * Order entity representation
 */
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub id: u64,
}

pub enum Status {
    Pending,
    Done,
}

// Parses input buffer with lifetime
pub fn parse_data<'a, 'b>(src: &'a str, tag: &'b str) -> &'a str {
    helper(src);
    src
}
"#;
    let rs_facts = Language::Rust.parse(rs_code);
    assert_eq!(
        rs_facts.defines,
        vec!["calculate_total", "Order", "Status", "parse_data"]
    );
    assert_eq!(rs_facts.docs.len(), 3);
    assert!(rs_facts.docs[0].1.contains("Calculates the total"));
    assert!(rs_facts.docs[0].1.contains("currency conversion"));
    assert!(rs_facts.docs[1].1.contains("Order entity"));
    assert!(rs_facts.docs[2]
        .1
        .contains("Parses input buffer with lifetime"));
    assert_eq!(rs_facts.calls.len(), 3);
    assert_eq!(
        rs_facts.calls[2],
        ("parse_data".to_string(), "helper".to_string(), false)
    );

    // 2. Go with standard // comments
    let go_code = r#"
package service

// ProcessOrder handles an incoming customer order.
// Validates inventory before persisting.
func (s *Server) ProcessOrder(id int) error {
    s.logger.Info(id)
    return validate(id)
}

/*
Config holds server settings
*/
type Config struct {
    Port int
}
"#;
    let go_facts = Language::Go.parse(go_code);
    assert_eq!(go_facts.defines, vec!["service", "ProcessOrder", "Config"]);
    assert_eq!(go_facts.docs.len(), 2);
    assert!(go_facts.docs[0].1.contains("ProcessOrder handles"));
    assert!(go_facts.docs[1].1.contains("Config holds"));

    // 3. Python with decorators and # comments & docstrings
    let py_code = r#"
# Manages user accounts and credentials
@dataclass
class UserManager:
    """Class docstring"""
    
    # Creates a new user record in the database
    @auth_required
    @rate_limit(10)
    async def create_user(self, name):
        validate_name(name)
        self.repo.save(name)
"#;
    let py_facts = Language::Python.parse(py_code);
    assert_eq!(py_facts.defines, vec!["UserManager", "create_user"]);
    assert_eq!(py_facts.docs.len(), 2);
    assert!(py_facts.docs[0].1.contains("Manages user accounts"));
    assert!(py_facts.docs[1].1.contains("Creates a new user record"));

    // 4. TypeScript with JSDoc and decorators
    let ts_code = r#"
/**
 * Represents an authenticated user profile
 */
export interface User {
    id: string;
}

// Fetches user profile from API
@observable
export async function fetchUser(id: string): Promise<User> {
    api.get(id);
    return parse(id);
}
"#;
    let ts_facts = Language::TypeScript.parse(ts_code);
    assert_eq!(ts_facts.defines, vec!["User", "fetchUser"]);
    assert_eq!(ts_facts.docs.len(), 2);
    assert!(ts_facts.docs[0].1.contains("Represents an authenticated"));
    assert!(ts_facts.docs[1].1.contains("Fetches user profile"));
}

#[test]
fn test_category_02_modern_systems() {
    // 1. Zig
    let zig_code = r#"
/// Allocates and initializes a memory pool
pub fn createPool(size: usize) !*Pool {
    return init(size);
}

// Point 2D coordinate representation
pub const Point = struct {
    x: f32,
    y: f32,
};
"#;
    let zig_facts = Language::Zig.parse(zig_code);
    assert_eq!(zig_facts.defines, vec!["createPool", "Point"]);
    assert_eq!(zig_facts.docs.len(), 2);
    assert!(zig_facts.docs[0].1.contains("Allocates and initializes"));
    assert!(zig_facts.docs[1].1.contains("Point 2D coordinate"));

    // 2. Nim
    let nim_code = r#"
## Calculates the fibonacci number
proc fib*(n: int): int =
  if n <= 1: return n
  return fib(n - 1) + fib(n - 2)

## Custom Vector type
type Vector* = object
  x, y: float
"#;
    let nim_facts = Language::Nim.parse(nim_code);
    assert_eq!(nim_facts.defines, vec!["fib", "Vector"]);
    assert_eq!(nim_facts.docs.len(), 2);
    assert!(nim_facts.docs[0].1.contains("Calculates the fibonacci"));

    // 3. Odin
    let odin_code = r#"
// Performs matrix multiplication
matrix_mult :: proc(a, b: Matrix) -> Matrix {
    return compute(a, b)
}

// Dynamic vector buffer
Vector :: struct {
    data: [^]f32,
}
"#;
    let odin_facts = Language::Odin.parse(odin_code);
    assert_eq!(odin_facts.defines, vec!["matrix_mult", "Vector"]);
    assert_eq!(odin_facts.docs.len(), 2);
    assert!(odin_facts.docs[0]
        .1
        .contains("Performs matrix multiplication"));
}

#[test]
fn test_category_03_functional() {
    // 1. Haskell
    let haskell_code = r#"
-- | Computes quicksort over list
quicksort :: Ord a => [a] -> [a]

{-
Container tree datatype
-}
data Tree a = Leaf | Node a (Tree a) (Tree a)
"#;
    let hs_facts = Language::Haskell.parse(haskell_code);
    assert_eq!(hs_facts.defines, vec!["quicksort", "Tree"]);
    assert_eq!(hs_facts.docs.len(), 2);
    assert!(hs_facts.docs[0].1.contains("Computes quicksort"));
    assert!(hs_facts.docs[1].1.contains("Container tree datatype"));

    // 2. Elixir
    let elixir_code = r#"
# Core business logic module
defmodule App.Accounts do
  @typedoc "Account identifier integer"
  
  # Validates positive integer id
  defguard is_valid_id(id) when is_integer(id) and id > 0

  # Creates a new user profile
  def create_user(params) do
    Repo.insert(params)
  end
end
"#;
    let ex_facts = Language::Elixir.parse(elixir_code);
    assert_eq!(
        ex_facts.defines,
        vec!["App.Accounts", "is_valid_id", "create_user"]
    );
    assert_eq!(ex_facts.docs.len(), 3);
    assert!(ex_facts.docs[0].1.contains("Core business logic"));
    assert!(ex_facts.docs[1].1.contains("Validates positive"));
    assert!(ex_facts.docs[2].1.contains("Creates a new user"));

    // 3. OCaml
    let ocaml_code = r#"
(** Evaluates mathematical expression *)
let rec eval expr =
  match expr with
  | Val v -> v

(** Result variant type *)
type result = Ok of int | Err of string
"#;
    let ml_facts = Language::OCaml.parse(ocaml_code);
    assert_eq!(ml_facts.defines, vec!["eval", "result"]);
    assert_eq!(ml_facts.docs.len(), 2);
    assert!(ml_facts.docs[0].1.contains("Evaluates mathematical"));
    assert!(ml_facts.docs[1].1.contains("Result variant"));
}

#[test]
fn test_category_04_devops_config() {
    // 1. HCL / Terraform
    let hcl_code = r#"
# Primary AWS VPC Network
resource "aws_vpc" "main" {
  cidr_block = "10.0.0.0/16"
}

// Global environment identifier
variable "environment" {
  type = string
}

# Health check verification
check "api_health" {
  assert {
    condition = true
  }
}
"#;
    let hcl_facts = Language::HclTerraform.parse(hcl_code);
    assert_eq!(hcl_facts.defines, vec!["main", "environment", "api_health"]);
    assert_eq!(hcl_facts.docs.len(), 3);
    assert!(hcl_facts.docs[0].1.contains("Primary AWS VPC"));
    assert!(hcl_facts.docs[1].1.contains("Global environment"));
    assert!(hcl_facts.docs[2].1.contains("Health check verification"));

    // 2. YAML
    let yaml_code = r#"
# Database connection settings
database:
  # Hostname of postgres
  host: localhost
  port: 5432
"#;
    let yaml_facts = Language::Yaml.parse(yaml_code);
    assert!(yaml_facts.defines.contains(&"database".to_string()));
    assert_eq!(yaml_facts.docs.len(), 2);

    // 3. TOML
    let toml_code = r#"
# Package metadata
[package]
name = "glasir"

# Web server port
port = 8080
"#;
    let toml_facts = Language::Toml.parse(toml_code);
    assert_eq!(toml_facts.defines, vec!["package", "name", "port"]);
    assert_eq!(toml_facts.docs.len(), 2);

    // 4. Dockerfile
    let docker_code = r#"
# Build compilation stage
FROM rust:1.80 AS builder

# Final minimal alpine runtime
FROM alpine:3.20
"#;
    let docker_facts = Language::Dockerfile.parse(docker_code);
    assert_eq!(docker_facts.defines, vec!["builder", "alpine:3.20"]);
    assert_eq!(docker_facts.docs.len(), 2);
}

#[test]
fn test_category_05_web_ui() {
    // 1. HTML
    let html_code = r#"
<!-- Main application container root -->
<div id="app" class="container">
  <!-- Search input component -->
  <input name="search-query" type="text" />
</div>
"#;
    let html_facts = Language::Html.parse(html_code);
    assert_eq!(html_facts.defines, vec!["id:app", "input:search-query"]);
    assert_eq!(html_facts.docs.len(), 2);
    assert!(html_facts.docs[0].1.contains("Main application container"));

    // 2. CSS
    let css_code = r#"
/* Primary call to action button */
.btn-primary {
  background: blue;
}

/* Site header banner */
#main-header {
  height: 60px;
}
"#;
    let css_facts = Language::Css.parse(css_code);
    assert_eq!(css_facts.defines, vec![".btn-primary", "#main-header"]);
    assert_eq!(css_facts.docs.len(), 2);
    assert!(css_facts.docs[0].1.contains("Primary call to action"));
}

#[test]
fn test_category_06_database() {
    // 1. SQL
    let sql_code = r#"
-- Stores registered customer accounts
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255)
);

-- Active subscribers analytical view
CREATE MATERIALIZED VIEW active_subscribers AS
SELECT * FROM users;
"#;
    let sql_facts = Language::Sql.parse(sql_code);
    assert_eq!(sql_facts.defines, vec!["users", "active_subscribers"]);
    assert_eq!(sql_facts.docs.len(), 2);
    assert!(sql_facts.docs[0].1.contains("Stores registered customer"));
    assert!(sql_facts
        .calls
        .iter()
        .any(|c| c.0 == "active_subscribers" && c.1 == "users"));

    // 2. Protobuf
    let proto_code = r#"
// Request payload for creating a profile
message CreateUserRequest {
    string name = 1;
}

// User account management RPC service
service UserService {
    rpc GetUser (UserQuery) returns (User);
}
"#;
    let proto_facts = Language::Protobuf.parse(proto_code);
    assert_eq!(
        proto_facts.defines,
        vec!["CreateUserRequest", "UserService", "GetUser"]
    );
    assert_eq!(proto_facts.docs.len(), 2);
}

#[test]
fn test_category_07_scripting() {
    // 1. Bash
    let bash_code = r#"
# Deploys application release artifacts
deploy_app() {
    echo "deploying"
}

# Runs database migrations
function run_migrations() {
    migrate
}
"#;
    let bash_facts = Language::Bash.parse(bash_code);
    assert_eq!(bash_facts.defines, vec!["deploy_app", "run_migrations"]);
    assert_eq!(bash_facts.docs.len(), 2);

    // 2. Perl
    let perl_code = r#"
# Processes incoming HTTP request
sub process_request {
    my ($self, $req) = @_;
    $self->log_info("handling");
}
"#;
    let perl_facts = Language::Perl.parse(perl_code);
    assert_eq!(perl_facts.defines, vec!["process_request"]);
    assert_eq!(perl_facts.docs.len(), 1);
    assert!(perl_facts.docs[0].1.contains("Processes incoming HTTP"));

    // 3. Lua
    let lua_code = r#"
--- Calculates square of number
function math.square(x)
    return x * x
end
"#;
    let lua_facts = Language::Lua.parse(lua_code);
    assert_eq!(lua_facts.defines, vec!["math.square"]);
    assert_eq!(lua_facts.docs.len(), 1);
}

#[test]
fn test_category_08_data_science() {
    // 1. R
    let r_code = r#"
#' Fits linear regression model
#' @param data data frame
fit_model <- function(data) {
    lm(y ~ x, data = data)
}
"#;
    let r_facts = Language::R.parse(r_code);
    assert_eq!(r_facts.defines, vec!["fit_model"]);
    assert_eq!(r_facts.docs.len(), 1);
    assert!(r_facts.docs[0].1.contains("Fits linear regression"));

    // 2. Julia
    let julia_code = r#"
# Computes eigen decomposition
function compute_eigen(matrix)
    eigen(matrix)
end

# 2D Point structure
mutable struct Point
    x::Float64
    y::Float64
end
"#;
    let julia_facts = Language::Julia.parse(julia_code);
    assert_eq!(julia_facts.defines, vec!["compute_eigen", "Point"]);
    assert_eq!(julia_facts.docs.len(), 2);

    // 3. OCaml
}

#[test]
fn test_category_09_contracts_hardware() {
    // 1. Solidity
    let sol_code = r#"
/// Custom user defined identifier
type AccountId is uint256;

/// ERC20 Token implementation contract
abstract contract MyToken {
    // Transfers tokens to recipient
    function transfer(address to, uint256 amount) public returns (bool) {
        emit Transfer(msg.sender, to, amount);
        return true;
    }
}
"#;
    let sol_facts = Language::Solidity.parse(sol_code);
    assert_eq!(sol_facts.defines, vec!["AccountId", "MyToken", "transfer"]);
    assert_eq!(sol_facts.docs.len(), 3);
    assert!(sol_facts.docs[0].1.contains("Custom user defined"));
    assert!(sol_facts.docs[1].1.contains("ERC20 Token"));

    // 2. Verilog
    let verilog_code = r#"
// UART Serial transmitter module
module uart_tx (
    input clk,
    output tx
);
endmodule
"#;
    let verilog_facts = Language::Verilog.parse(verilog_code);
    assert_eq!(verilog_facts.defines, vec!["uart_tx"]);
    assert_eq!(verilog_facts.docs.len(), 1);
    assert!(verilog_facts.docs[0].1.contains("UART Serial transmitter"));
}

#[test]
fn test_category_10_documents() {
    // 1. Markdown
    let md_code = r#"
<!-- Document main title -->
# Project Glasir

## Architecture Overview
"#;
    let md_facts = Language::Markdown.parse(md_code);
    assert_eq!(
        md_facts.defines,
        vec!["h1:Project Glasir", "h2:Architecture Overview"]
    );
    assert_eq!(md_facts.docs.len(), 1);
    assert!(md_facts.docs[0].1.contains("Document main title"));

    // 2. Typst
    let typst_code = r#"
// Document heading
= Introduction

// Primary theme color
#let primary_color = rgb("1e88e5")
"#;
    let typst_facts = Language::Typst.parse(typst_code);
    // Both readings are merged — `#let` bindings and their calls from the rule
    // path, headings from the line scanner — so the order is code first,
    // structure after. Asserted as a set: the order is an implementation
    // detail of the merge, the presence of both is the contract.
    assert!(typst_facts.defines.contains(&"h1:Introduction".to_string()));
    assert!(typst_facts.defines.contains(&"primary_color".to_string()));
    assert_eq!(typst_facts.defines.len(), 2);
    assert_eq!(typst_facts.docs.len(), 2);
    assert!(typst_facts
        .docs
        .iter()
        .any(|(_, d)| d.contains("Document heading")));
    assert!(typst_facts
        .docs
        .iter()
        .any(|(_, d)| d.contains("Primary theme color")));

    // 3. LaTeX
    let latex_code = r#"
% Top-level introduction section
\section{Introduction}

% Helper definition
\newcommand{\myVector}[1]{\mathbf{#1}}
"#;
    let latex_facts = Language::Latex.parse(latex_code);
    assert!(latex_facts
        .defines
        .contains(&"section:Introduction".to_string()));
    assert!(latex_facts.defines.contains(&"myVector".to_string()));
    assert!(latex_facts
        .docs
        .iter()
        .any(|(n, d)| n == "section:Introduction" && d.contains("Top-level introduction section")));
}

#[test]
fn test_resilience_and_zero_panic() {
    let malformed_inputs = vec![
        "",
        "   \n\t\r\n  ",
        "\"",
        "\"\"\"",
        "'''",
        "/* unclosed comment",
        "{- unclosed haskell",
        "(* unclosed ocaml",
        "--[[ unclosed lua",
        "#[ unclosed nim",
        "#= unclosed julia",
        "=pod unclosed perl",
        "<!-- unclosed html",
        "\"trailing escape \\",
        "\"\"\"trailing triple escape \\",
        "pub fn (",
        "def (",
        "class :",
        "func (",
        "let rec = ;",
        "CREATE TABLE",
        "resource",
        "@#$%^&*()_+-=[]{}|;':\",./<>?",
        "🦀 🚀 📦 // Unicode comment on emoji",
    ];

    let all_languages = vec![
        Language::Rust,
        Language::Go,
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Java,
        Language::CSharp,
        Language::Ruby,
        Language::Php,
        Language::Groovy,
        Language::Vb,
        Language::Cobol,
        Language::Zig,
        Language::Nim,
        Language::Odin,
        Language::C,
        Language::Cpp,
        Language::Swift,
        Language::Ada,
        Language::D,
        Language::Wat,
        Language::Haskell,
        Language::Elixir,
        Language::OCaml,
        Language::Scala,
        Language::Kotlin,
        Language::Erlang,
        Language::FSharp,
        Language::Clojure,
        Language::Elm,
        Language::Gleam,
        Language::PureScript,
        Language::Lisp,
        Language::Lean,
        Language::HclTerraform,
        Language::Yaml,
        Language::Toml,
        Language::Dockerfile,
        Language::Nix,
        Language::Json,
        Language::Html,
        Language::Css,
        Language::Dart,
        Language::Vue,
        Language::Svelte,
        Language::Xml,
        Language::Sql,
        Language::Protobuf,
        Language::GraphQL,
        Language::Thrift,
        Language::FlatBuffers,
        Language::CapnProto,
        Language::Cypher,
        Language::Bash,
        Language::Perl,
        Language::Lua,
        Language::PowerShell,
        Language::GdScript,
        Language::Batch,
        Language::Fish,
        Language::R,
        Language::Julia,
        Language::Matlab,
        Language::Mojo,
        Language::Fortran,
        Language::Solidity,
        Language::Verilog,
        Language::Vhdl,
        Language::Shader,
        Language::Markdown,
        Language::Typst,
        Language::Latex,
    ];

    for lang in all_languages {
        for input in &malformed_inputs {
            let facts = lang.parse(input);
            // Must never panic and must produce valid FileFacts
            assert!(facts.defines.len() <= input.len());
        }
    }
}

#[test]
fn test_extension_dispatch_all_languages() {
    let cases = vec![
        ("rs", Language::Rust),
        ("go", Language::Go),
        ("py", Language::Python),
        ("pyi", Language::Python),
        ("ts", Language::TypeScript),
        ("tsx", Language::TypeScript),
        ("js", Language::JavaScript),
        ("jsx", Language::JavaScript),
        ("java", Language::Java),
        ("cs", Language::CSharp),
        ("rb", Language::Ruby),
        ("rake", Language::Ruby),
        ("php", Language::Php),
        ("groovy", Language::Groovy),
        ("gradle", Language::Groovy),
        ("vb", Language::Vb),
        ("vbs", Language::Vb),
        ("cob", Language::Cobol),
        ("cbl", Language::Cobol),
        ("zig", Language::Zig),
        ("nim", Language::Nim),
        ("odin", Language::Odin),
        ("c", Language::C),
        ("h", Language::C),
        ("cpp", Language::Cpp),
        ("cc", Language::Cpp),
        ("cxx", Language::Cpp),
        ("c++", Language::Cpp),
        ("hpp", Language::Cpp),
        ("hh", Language::Cpp),
        ("hxx", Language::Cpp),
        ("swift", Language::Swift),
        ("adb", Language::Ada),
        ("ads", Language::Ada),
        ("d", Language::D),
        ("wat", Language::Wat),
        ("wast", Language::Wat),
        ("hs", Language::Haskell),
        ("ex", Language::Elixir),
        ("ml", Language::OCaml),
        ("scala", Language::Scala),
        ("sc", Language::Scala),
        ("kt", Language::Kotlin),
        ("kts", Language::Kotlin),
        ("erl", Language::Erlang),
        ("hrl", Language::Erlang),
        ("fs", Language::FSharp),
        ("fsi", Language::FSharp),
        ("clj", Language::Clojure),
        ("edn", Language::Clojure),
        ("elm", Language::Elm),
        ("gleam", Language::Gleam),
        ("purs", Language::PureScript),
        ("lisp", Language::Lisp),
        ("lsp", Language::Lisp),
        ("scm", Language::Lisp),
        ("lean", Language::Lean),
        ("tf", Language::HclTerraform),
        ("yaml", Language::Yaml),
        ("yml", Language::Yaml),
        ("toml", Language::Toml),
        ("dockerfile", Language::Dockerfile),
        ("nix", Language::Nix),
        ("json", Language::Json),
        ("html", Language::Html),
        ("css", Language::Css),
        ("dart", Language::Dart),
        ("vue", Language::Vue),
        ("svelte", Language::Svelte),
        ("xml", Language::Xml),
        ("svg", Language::Xml),
        ("sql", Language::Sql),
        ("proto", Language::Protobuf),
        ("graphql", Language::GraphQL),
        ("gql", Language::GraphQL),
        ("thrift", Language::Thrift),
        ("fbs", Language::FlatBuffers),
        ("capnp", Language::CapnProto),
        ("cql", Language::Cypher),
        ("sh", Language::Bash),
        ("pl", Language::Perl),
        ("lua", Language::Lua),
        ("ps1", Language::PowerShell),
        ("gd", Language::GdScript),
        ("bat", Language::Batch),
        ("fish", Language::Fish),
        ("r", Language::R),
        ("jl", Language::Julia),
        ("m", Language::Matlab),
        ("matlab", Language::Matlab),
        ("mojo", Language::Mojo),
        ("f90", Language::Fortran),
        ("for", Language::Fortran),
        ("sol", Language::Solidity),
        ("v", Language::Verilog),
        ("vhd", Language::Vhdl),
        ("vhdl", Language::Vhdl),
        ("wgsl", Language::Shader),
        ("glsl", Language::Shader),
        ("hlsl", Language::Shader),
        ("md", Language::Markdown),
        ("typ", Language::Typst),
        ("tex", Language::Latex),
        ("latex", Language::Latex),
        ("sty", Language::Latex),
        ("cls", Language::Latex),
    ];

    for (ext, expected_lang) in cases {
        let lang = Language::from_extension(ext);
        assert_eq!(
            lang,
            Some(expected_lang),
            "Extension {} should map to {:?}",
            ext,
            expected_lang
        );
    }
}

#[test]
fn test_call_graph_and_receiver_extraction() {
    // Rust call extraction
    let rs = r#"
fn run() {
    local_func();
    service.remote_call();
    Database::connect();
}
"#;
    let facts = Language::Rust.parse(rs);
    assert_eq!(facts.defines, vec!["run"]);
    assert_eq!(facts.calls.len(), 3);
    assert_eq!(
        facts.calls[0],
        ("run".to_string(), "local_func".to_string(), false)
    );
    assert_eq!(
        facts.calls[1],
        ("run".to_string(), "remote_call".to_string(), true)
    );
    assert_eq!(
        facts.calls[2],
        ("run".to_string(), "connect".to_string(), true)
    );
}

#[test]
fn test_body_comments_accumulation() {
    // 1. Rust body comments: Doc comment above + comments inside the body must be merged!
    let rs = r#"
/// High-level payment processor
pub fn process_payment(amount: u64) -> bool {
    // Step 1: Validate customer credit balance
    let valid = check_balance(amount);
    // Step 2: Invoke billing gateway with timeout
    let billed = gateway.charge(amount);
    valid && billed
}
"#;
    let facts = Language::Rust.parse(rs);
    assert_eq!(facts.defines, vec!["process_payment"]);
    assert_eq!(facts.docs.len(), 1);
    let doc = &facts.docs[0].1;
    assert!(doc.contains("High-level payment processor"));
    assert!(doc.contains("Step 1: Validate customer credit balance"));
    assert!(doc.contains("Step 2: Invoke billing gateway with timeout"));
    // Verify full length (over 68 characters, not just the 15 char head)
    assert!(
        doc.len() >= 68,
        "Doc length was {} but expected >= 68",
        doc.len()
    );

    // 2. Python body comments and docstrings
    let py = r#"
# Core billing controller
class BillingController:
    """Class docstring"""
    
    # Executes monthly invoice run
    def run_invoices(self):
        # Step A: collect pending accounts
        accounts = self.get_pending()
        # Step B: dispatch payment tasks
        self.dispatch(accounts)
"#;
    let py_facts = Language::Python.parse(py);
    assert_eq!(py_facts.defines, vec!["BillingController", "run_invoices"]);
    assert_eq!(py_facts.docs.len(), 2);
    let class_doc = &py_facts.docs[0].1;
    let method_doc = &py_facts.docs[1].1;
    assert!(class_doc.contains("Core billing controller"));
    assert!(method_doc.contains("Executes monthly invoice run"));
    assert!(method_doc.contains("Step A: collect pending accounts"));
    assert!(method_doc.contains("Step B: dispatch payment tasks"));
    assert!(
        method_doc.len() >= 68,
        "Method doc length was {} but expected >= 68",
        method_doc.len()
    );
}

#[test]
fn test_has_receiver_self_this_disambiguation() {
    // 1. Rust: self.f() and Self::f() must have has_receiver: false (local resolution)
    // while obj.f() and Module::f() must have has_receiver: true
    let rs = r#"
struct Checkout { total: u32 }
impl Checkout {
    fn pay(&self) -> bool {
        self.charge() && Self::log_info("paid") && gateway.process(self.total)
    }
}
"#;
    let facts = Language::Rust.parse(rs);
    assert_eq!(facts.defines, vec!["Checkout", "pay"]);
    // self.charge() -> false
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "charge" && !c.2),
        "self.charge() must have has_receiver=false"
    );
    // Self::log_info() -> false
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "log_info" && !c.2),
        "Self::log_info() must have has_receiver=false"
    );
    // gateway.process() -> true
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "process" && c.2),
        "gateway.process() must have has_receiver=true"
    );

    // 2. TypeScript/JavaScript: this.f() must have has_receiver: false, api.f() must have has_receiver: true
    let ts = r#"
class Checkout {
    pay(): boolean {
        this.charge();
        api.process();
        return true;
    }
}
"#;
    let ts_facts = Language::TypeScript.parse(ts);
    assert!(
        ts_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "charge" && !c.2),
        "this.charge() must have has_receiver=false"
    );
    assert!(
        ts_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "process" && c.2),
        "api.process() must have has_receiver=true"
    );

    // 3. Python: self.f() must have has_receiver: false, repo.f() must have has_receiver: true
    let py = r#"
class Checkout:
    def pay(self):
        self.charge()
        repo.save()
"#;
    let py_facts = Language::Python.parse(py);
    assert!(
        py_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "charge" && !c.2),
        "self.charge() in Python must have has_receiver=false"
    );
    assert!(
        py_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "save" && c.2),
        "repo.save() in Python must have has_receiver=true"
    );
}

#[test]
fn test_edges_haskell_ocaml_r_sql_hcl() {
    // 1. Haskell juxtaposition call extraction
    let hs = r#"
pay :: Cart -> Int
pay c = charge (total c)

charge :: Int -> Int
charge t = t
"#;
    let hs_facts = Language::Haskell.parse(hs);
    assert!(hs_facts.defines.contains(&"pay".to_string()));
    assert!(
        hs_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "charge" && !c.2),
        "Haskell: pay must call charge, got {:?}",
        hs_facts.calls
    );
    assert!(
        hs_facts
            .calls
            .iter()
            .any(|c| c.0 == "pay" && c.1 == "total" && !c.2),
        "Haskell: pay must call total, got {:?}",
        hs_facts.calls
    );

    // 2. OCaml juxtaposition and pipeline call extraction
    let ml = r#"
let eval expr =
  let v = compute (transform expr) in
  List.map validate v
"#;
    let ml_facts = Language::OCaml.parse(ml);
    assert!(ml_facts.defines.contains(&"eval".to_string()));
    assert!(
        ml_facts
            .calls
            .iter()
            .any(|c| c.0 == "eval" && c.1 == "compute" && !c.2),
        "OCaml: eval must call compute, got {:?}",
        ml_facts.calls
    );
    assert!(
        ml_facts
            .calls
            .iter()
            .any(|c| c.0 == "eval" && c.1 == "transform" && !c.2),
        "OCaml: eval must call transform, got {:?}",
        ml_facts.calls
    );
    assert!(
        ml_facts
            .calls
            .iter()
            .any(|c| c.0 == "eval" && c.1 == "map" && c.2),
        "OCaml: eval must call List.map with receiver, got {:?}",
        ml_facts.calls
    );

    // 3. R function calls, method calls, and pipelines
    let r = r#"
fit_model <- function(df) {
    clean_data(df)
    obj$transform(df)
}
"#;
    let r_facts = Language::R.parse(r);
    assert!(r_facts.defines.contains(&"fit_model".to_string()));
    assert!(
        r_facts
            .calls
            .iter()
            .any(|c| c.0 == "fit_model" && c.1 == "clean_data" && !c.2),
        "R: fit_model must call clean_data, got {:?}",
        r_facts.calls
    );
    assert!(
        r_facts
            .calls
            .iter()
            .any(|c| c.0 == "fit_model" && c.1 == "transform" && c.2),
        "R: fit_model must call obj$transform with receiver, got {:?}",
        r_facts.calls
    );

    // 4. SQL function and procedure calls
    let sql = r#"
CREATE FUNCTION calculate_tax(subtotal NUMERIC) RETURNS NUMERIC AS $$
BEGIN
    CALL audit_log(subtotal);
    RETURN round_val(subtotal * 0.19);
END;
$$;
"#;
    let sql_facts = Language::Sql.parse(sql);
    assert!(sql_facts.defines.contains(&"calculate_tax".to_string()));
    assert!(
        sql_facts
            .calls
            .iter()
            .any(|c| c.0 == "calculate_tax" && c.1 == "audit_log"),
        "SQL: calculate_tax must call audit_log, got {:?}",
        sql_facts.calls
    );
    assert!(
        sql_facts
            .calls
            .iter()
            .any(|c| c.0 == "calculate_tax" && c.1 == "round_val"),
        "SQL: calculate_tax must call round_val, got {:?}",
        sql_facts.calls
    );

    // 5. HCL/Terraform functions and references
    let hcl = r#"
resource "aws_s3_bucket" "main" {
  bucket = templatefile("policy.json", { env = "prod" })
}
"#;
    let hcl_facts = Language::HclTerraform.parse(hcl);
    assert!(hcl_facts.defines.contains(&"main".to_string()));
    assert!(
        hcl_facts
            .calls
            .iter()
            .any(|c| c.0 == "main" && c.1 == "templatefile" && !c.2),
        "HCL: resource must call templatefile, got {:?}",
        hcl_facts.calls
    );
}

#[test]
fn test_auftrag_five_points() {
    // 1. Struct field doc isolation (18 chars, only struct doc, no field docs)
    let rs_code = r#"
/// Kurze Struct-Doku.
pub struct S {
    /// Feld-Doku eins.
    pub a: String,
    /// Feld-Doku zwei.
    pub b: String,
}
"#;
    let rs_facts = Language::Rust.parse(rs_code);
    assert_eq!(rs_facts.defines, vec!["S"]);
    assert_eq!(rs_facts.docs.len(), 1);
    assert_eq!(rs_facts.docs[0].1, "Kurze Struct-Doku.");
    assert_eq!(rs_facts.docs[0].1.len(), 18);

    // 2. Juxtaposition argument handling in Haskell (pay -> charge only, not c)
    let hs_code = "pay c = charge c\n";
    let hs_facts = Language::Haskell.parse(hs_code);
    assert_eq!(hs_facts.defines, vec!["pay"]);
    assert_eq!(
        hs_facts.calls,
        vec![("pay".to_string(), "charge".to_string(), false)]
    );

    // 3. OCaml top-level `let` definitions (2 defines: pay and charge, 1 call: pay -> charge)
    let ml_code = "let pay c = charge c\nlet charge t = t\n";
    let ml_facts = Language::OCaml.parse(ml_code);
    assert_eq!(ml_facts.defines, vec!["pay", "charge"]);
    assert_eq!(
        ml_facts.calls,
        vec![("pay".to_string(), "charge".to_string(), false)]
    );

    // 4. Solidity: returns keyword excluded from calls, bare name Checkout
    let sol_code = r#"
contract Checkout {
    function pay(uint c) public returns (uint) { return charge(c); }
    function charge(uint t) internal returns (uint) { return t; }
}
"#;
    let sol_facts = Language::Solidity.parse(sol_code);
    assert_eq!(sol_facts.defines, vec!["Checkout", "pay", "charge"]);
    assert_eq!(
        sol_facts.calls,
        vec![("pay".to_string(), "charge".to_string(), false)]
    );

    // 5. SQL and HCL edge extraction & bare symbols
    let sql_code = r#"
CREATE TABLE contacts (id INT PRIMARY KEY, name TEXT);
CREATE VIEW active AS SELECT * FROM contacts WHERE id > 0;
"#;
    let sql_facts = Language::Sql.parse(sql_code);
    assert_eq!(sql_facts.defines, vec!["contacts", "active"]);
    assert_eq!(
        sql_facts.calls,
        vec![("active".to_string(), "contacts".to_string(), false)]
    );

    let hcl_code = r#"
resource "aws_instance" "web" { ami = var.ami_id }
module "vpc" { source = "./vpc" }
"#;
    let hcl_facts = Language::HclTerraform.parse(hcl_code);
    assert_eq!(hcl_facts.defines, vec!["web", "vpc"]);
    assert!(
        hcl_facts
            .calls
            .iter()
            .any(|c| c.0 == "web" && c.1 == "ami_id" && c.2),
        "HCL: web must have receiver edge to ami_id, got: {:?}",
        hcl_facts.calls
    );
}

#[test]
fn test_raw_strings_and_isolation() {
    // 1. Exact user test case: raw string containing a function definition and comment
    let rs = r###"
fn a() {
    let s = r#"
        fn fake() {
            // Kommentar im Raw-String
    "#;
    // Kommentar in a.
}
fn b() { /* Kommentar in b. */ }
"###;
    let facts = Language::Rust.parse(rs);
    assert_eq!(
        facts.defines,
        vec!["a", "b"],
        "Must not contain phantom definition fake"
    );

    let a_doc = facts
        .docs
        .iter()
        .find(|(name, _)| name == "a")
        .map(|(_, d)| d.as_str());
    let b_doc = facts
        .docs
        .iter()
        .find(|(name, _)| name == "b")
        .map(|(_, d)| d.as_str());

    assert_eq!(a_doc, Some("Kommentar in a."));
    assert_eq!(b_doc, Some("Kommentar in b."));

    // 2. Odd number of quotes inside raw strings
    let rs_odd = r###"
fn process() {
    let single_quote = r#" single " quote // 1 quote (odd) inside "#;
    let three_quotes = r#" three """ quotes // 3 quotes (odd) inside "#;
    let five_quotes = r#" five " " " " " quotes // 5 quotes (odd) inside "#;
    let multi_hash = r##" multi-hash with "# and """ and " // 2 hashes inside "##;
    // Real body comment for process.
}
fn after_odd() {
    // Comment for after_odd.
}
"###;
    let facts_odd = Language::Rust.parse(rs_odd);
    assert_eq!(facts_odd.defines, vec!["process", "after_odd"]);
    let proc_doc = facts_odd
        .docs
        .iter()
        .find(|(name, _)| name == "process")
        .map(|(_, d)| d.as_str());
    let after_doc = facts_odd
        .docs
        .iter()
        .find(|(name, _)| name == "after_odd")
        .map(|(_, d)| d.as_str());
    assert_eq!(proc_doc, Some("Real body comment for process."));
    assert_eq!(after_doc, Some("Comment for after_odd."));

    // 3. Byte raw strings (br#"..."#) and C raw strings (cr#"..."#)
    let rs_byte = r###"
fn parse_buffers() {
    let b = br#"
        fn fake_byte_fn() {
            // fake comment
    "#;
    let c = cr#"
        fn fake_c_fn() {
            // fake c comment
    "#;
    // Genuine comment for parse_buffers.
}
"###;
    let facts_byte = Language::Rust.parse(rs_byte);
    assert_eq!(facts_byte.defines, vec!["parse_buffers"]);
    let buf_doc = facts_byte
        .docs
        .iter()
        .find(|(name, _)| name == "parse_buffers")
        .map(|(_, d)| d.as_str());
    assert_eq!(buf_doc, Some("Genuine comment for parse_buffers."));

    // 4. Python raw multiline strings
    let py = r###"
def real_fn():
    s = r"""
        def fake_py_fn():
            # inside fake
    """
    # Real comment for real_fn
def second_fn():
    # Real comment for second_fn
    pass
"###;
    let facts_py = Language::Python.parse(py);
    assert_eq!(facts_py.defines, vec!["real_fn", "second_fn"]);
    let r1 = facts_py
        .docs
        .iter()
        .find(|(name, _)| name == "real_fn")
        .map(|(_, d)| d.as_str());
    let r2 = facts_py
        .docs
        .iter()
        .find(|(name, _)| name == "second_fn")
        .map(|(_, d)| d.as_str());
    assert_eq!(r1, Some("Real comment for real_fn"));
    assert_eq!(r2, Some("Real comment for second_fn"));
}

#[test]
fn test_string_line_continuation_and_zero_calls() {
    // 1. String line continuation with `\`
    let rs = r###"
fn demo() {
    let py = "\
class Checkout:
    def main():
        pay()
";
    let _ = py;
}
"###;
    let facts = Language::Rust.parse(rs);
    assert_eq!(facts.defines, vec!["demo"]);
    assert!(
        facts.calls.is_empty(),
        "Must find 0 calls inside demo, got: {:?}",
        facts.calls
    );
}

#[test]
fn test_tuple_field_receiver_call() {
    // 2. Call via tuple field index: busy.0.publish(&source, compacted);
    let rs = r###"
fn outer() {
    let busy = Busy(graph);
    busy.0.publish(&source, compacted);
    self.0.flush();
    self.direct();
}
"###;
    let facts = Language::Rust.parse(rs);
    assert_eq!(facts.defines, vec!["outer"]);

    // busy.0.publish -> has_receiver = true
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "outer" && c.1 == "publish" && c.2),
        "busy.0.publish must have has_receiver=true, got: {:?}",
        facts.calls
    );
    // self.0.flush -> has_receiver = true
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "outer" && c.1 == "flush" && c.2),
        "self.0.flush must have has_receiver=true, got: {:?}",
        facts.calls
    );
    // self.direct -> has_receiver = false
    assert!(
        facts
            .calls
            .iter()
            .any(|c| c.0 == "outer" && c.1 == "direct" && !c.2),
        "self.direct must have has_receiver=false, got: {:?}",
        facts.calls
    );
}

#[test]
fn test_comprehensive_auftrag_edge_cases() {
    // 1. Haskell chained functions and higher-order calls
    let hs = r#"
processOrder :: Order -> Result
processOrder o = logEvent (validateOrder o)

calcTax :: Double -> Double
calcTax amt = roundVal (amt * 0.19)
"#;
    let hs_facts = Language::Haskell.parse(hs);
    assert_eq!(hs_facts.defines, vec!["processOrder", "calcTax"]);
    assert!(hs_facts
        .calls
        .iter()
        .any(|c| c.0 == "processOrder" && c.1 == "logEvent" && !c.2));
    assert!(hs_facts
        .calls
        .iter()
        .any(|c| c.0 == "processOrder" && c.1 == "validateOrder" && !c.2));
    assert!(hs_facts
        .calls
        .iter()
        .any(|c| c.0 == "calcTax" && c.1 == "roundVal" && !c.2));
    assert!(
        !hs_facts.calls.iter().any(|c| c.1 == "o" || c.1 == "amt"),
        "Args o and amt must not be callees"
    );

    // 2. OCaml multiple top-level let bindings and local let bindings
    let ml = r#"
let add a b = compute a b
let mult x y =
  let p = multiply x y in
  display p
let identity x = x
"#;
    let ml_facts = Language::OCaml.parse(ml);
    assert_eq!(ml_facts.defines, vec!["add", "mult", "identity"]);
    assert!(ml_facts
        .calls
        .iter()
        .any(|c| c.0 == "add" && c.1 == "compute" && !c.2));
    assert!(ml_facts
        .calls
        .iter()
        .any(|c| c.0 == "mult" && c.1 == "multiply" && !c.2));
    assert!(ml_facts
        .calls
        .iter()
        .any(|c| c.0 == "mult" && c.1 == "display" && !c.2));
    assert!(
        !ml_facts.calls.iter().any(|c| c.0 == "identity"),
        "Identity must have 0 calls"
    );

    // 3. Solidity contract with multiple returns and types
    let sol = r#"
contract PaymentGateway {
    struct Config {
        uint256 fee;
    }
    function process(uint256 amount) public returns (bool, uint256) {
        return (charge(amount), calculateFee(amount));
    }
    function charge(uint256 a) internal returns (bool) {
        return true;
    }
    function calculateFee(uint256 a) internal pure returns (uint256) {
        return a;
    }
}
"#;
    let sol_facts = Language::Solidity.parse(sol);
    assert_eq!(
        sol_facts.defines,
        vec![
            "PaymentGateway",
            "Config",
            "process",
            "charge",
            "calculateFee"
        ]
    );
    assert!(!sol_facts
        .calls
        .iter()
        .any(|c| c.1 == "returns" || c.1 == "public" || c.1 == "internal" || c.1 == "pure"));
    assert!(sol_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "charge" && !c.2));
    assert!(sol_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "calculateFee" && !c.2));

    // 4. SQL schema with foreign keys and view references
    let sql = r#"
CREATE TABLE accounts (id INT PRIMARY KEY, balance NUMERIC);
CREATE TABLE transactions (id INT PRIMARY KEY, account_id INT REFERENCES accounts(id));
CREATE VIEW large_transactions AS SELECT * FROM transactions WHERE amount > 1000;
"#;
    let sql_facts = Language::Sql.parse(sql);
    assert_eq!(
        sql_facts.defines,
        vec!["accounts", "transactions", "large_transactions"]
    );
    assert!(sql_facts
        .calls
        .iter()
        .any(|c| c.0 == "transactions" && c.1 == "accounts"));
    assert!(sql_facts
        .calls
        .iter()
        .any(|c| c.0 == "large_transactions" && c.1 == "transactions"));

    // 5. HCL resources and dependencies
    let hcl = r#"
variable "region" { default = "eu-central-1" }
resource "aws_vpc" "vpc" { cidr_block = var.region }
resource "aws_subnet" "subnet" { vpc_id = aws_vpc.vpc.id }
"#;
    let hcl_facts = Language::HclTerraform.parse(hcl);
    assert_eq!(hcl_facts.defines, vec!["region", "vpc", "subnet"]);
    assert!(hcl_facts
        .calls
        .iter()
        .any(|c| c.0 == "vpc" && c.1 == "region" && c.2));
    assert!(hcl_facts
        .calls
        .iter()
        .any(|c| c.0 == "subnet" && c.1 == "vpc" && c.2));
}

#[test]
fn test_ten_new_languages_full_suite() {
    // 1. Java: Package, Class, Interface, Enum, Record, Method, Annotations, Calls
    let java = r#"
package com.example.service;

/**
 * Account management service
 */
@Service
@Transactional
public class AccountService {
    // Current account balance field (must NOT leak into class doc)
    private double balance;

    /**
     * Transfers money between accounts
     */
    public boolean transfer(Account from, Account to, double amount) {
        // Step 1: validate funds
        boolean ok = this.checkFunds(from, amount);
        // Step 2: process debit
        super.auditTransfer(from, to);
        // Step 3: external notification
        notifier.sendAlert(to, amount);
        return ok;
    }

    private boolean checkFunds(Account acc, double amount) {
        return acc.getBalance() >= amount;
    }
}
"#;
    let java_facts = Language::Java.parse(java);
    assert_eq!(
        java_facts.defines,
        vec![
            "com.example.service",
            "AccountService",
            "transfer",
            "checkFunds"
        ]
    );
    let cls_doc = java_facts
        .docs
        .iter()
        .find(|(n, _)| n == "AccountService")
        .map(|(_, d)| d.as_str());
    assert_eq!(cls_doc, Some("Account management service"));
    let m_doc = java_facts
        .docs
        .iter()
        .find(|(n, _)| n == "transfer")
        .map(|(_, d)| d.as_str());
    assert!(m_doc.unwrap().contains("Transfers money between accounts"));
    assert!(m_doc.unwrap().contains("Step 1: validate funds"));
    assert!(m_doc.unwrap().contains("Step 2: process debit"));
    assert!(m_doc.unwrap().contains("Step 3: external notification"));
    // this.checkFunds -> has_receiver: false
    assert!(java_facts
        .calls
        .iter()
        .any(|c| c.0 == "transfer" && c.1 == "checkFunds" && !c.2));
    // super.auditTransfer -> has_receiver: false
    assert!(java_facts
        .calls
        .iter()
        .any(|c| c.0 == "transfer" && c.1 == "auditTransfer" && !c.2));
    // notifier.sendAlert -> has_receiver: true
    assert!(java_facts
        .calls
        .iter()
        .any(|c| c.0 == "transfer" && c.1 == "sendAlert" && c.2));

    // 2. C: Macro, Struct, Enum, Function, Calls
    let c = r#"
#define MAX_BUFFER 4096

/**
 * Socket configuration struct
 */
struct socket_cfg {
    int fd;
    // port field doc
    int port;
};

/**
 * Initializes network socket connection
 */
int init_socket(struct socket_cfg *cfg) {
    // Step 1: allocate socket descriptor
    int sock = create_socket();
    // Step 2: configure options
    set_options(sock);
    // Step 3: remote logging
    logger->log_connect(sock);
    return sock;
}
"#;
    let c_facts = Language::C.parse(c);
    assert_eq!(
        c_facts.defines,
        vec!["MAX_BUFFER", "socket_cfg", "init_socket"]
    );
    let s_doc = c_facts
        .docs
        .iter()
        .find(|(n, _)| n == "socket_cfg")
        .map(|(_, d)| d.as_str());
    assert_eq!(s_doc, Some("Socket configuration struct"));
    let f_doc = c_facts
        .docs
        .iter()
        .find(|(n, _)| n == "init_socket")
        .map(|(_, d)| d.as_str());
    assert!(f_doc
        .unwrap()
        .contains("Initializes network socket connection"));
    assert!(f_doc
        .unwrap()
        .contains("Step 1: allocate socket descriptor"));
    assert!(f_doc.unwrap().contains("Step 2: configure options"));
    assert!(f_doc.unwrap().contains("Step 3: remote logging"));
    assert!(c_facts
        .calls
        .iter()
        .any(|c| c.0 == "init_socket" && c.1 == "create_socket" && !c.2));
    assert!(c_facts
        .calls
        .iter()
        .any(|c| c.0 == "init_socket" && c.1 == "set_options" && !c.2));
    assert!(c_facts
        .calls
        .iter()
        .any(|c| c.0 == "init_socket" && c.1 == "log_connect" && c.2));

    // 3. C++: Class, Namespace, Methods, this-> disambiguation
    let cpp = r#"
#define DEFAULT_TIMEOUT 30

namespace Net {
    /**
     * HTTP client implementation
     */
    class HttpClient {
        int timeout;
    public:
        /**
         * Sends HTTP GET request
         */
        Response get(const std::string &url) {
            // Validate URL format
            this->validate(url);
            // Execute request via socket
            return socket.send(url);
        }

        void validate(const std::string &url) {
            local_check(url);
        }
    };
}
"#;
    let cpp_facts = Language::Cpp.parse(cpp);
    assert_eq!(
        cpp_facts.defines,
        vec!["DEFAULT_TIMEOUT", "Net", "HttpClient", "get", "validate"]
    );
    let get_doc = cpp_facts
        .docs
        .iter()
        .find(|(n, _)| n == "get")
        .map(|(_, d)| d.as_str());
    assert!(get_doc.unwrap().contains("Sends HTTP GET request"));
    assert!(get_doc.unwrap().contains("Validate URL format"));
    assert!(get_doc.unwrap().contains("Execute request via socket"));
    // this->validate -> has_receiver: false
    assert!(cpp_facts
        .calls
        .iter()
        .any(|c| c.0 == "get" && c.1 == "validate" && !c.2));
    // socket.send -> has_receiver: true
    assert!(cpp_facts
        .calls
        .iter()
        .any(|c| c.0 == "get" && c.1 == "send" && c.2));

    // 4. C#: Namespace, Class, Attributes, base/this calls
    let cs = r#"
namespace Core.Services;

/// <summary>
/// Payment processor service
/// </summary>
[ApiController]
[Route("api/[controller]")]
public class PaymentService : BaseService {
    /// <summary>
    /// Executes transaction
    /// </summary>
    [HttpPost("pay")]
    public async Task<bool> PayAsync(PaymentReq req) {
        // Step A: Base initialization
        base.InitContext();
        // Step B: Local verification
        this.VerifyRequest(req);
        // Step C: Remote charge
        return await gateway.ChargeAsync(req);
    }

    private void VerifyRequest(PaymentReq req) {
        CheckValid(req);
    }
}
"#;
    let cs_facts = Language::CSharp.parse(cs);
    assert_eq!(
        cs_facts.defines,
        vec![
            "Core.Services",
            "PaymentService",
            "PayAsync",
            "VerifyRequest"
        ]
    );
    let pay_doc = cs_facts
        .docs
        .iter()
        .find(|(n, _)| n == "PayAsync")
        .map(|(_, d)| d.as_str());
    assert!(pay_doc.unwrap().contains("Executes transaction"));
    assert!(pay_doc.unwrap().contains("Step A: Base initialization"));
    assert!(pay_doc.unwrap().contains("Step B: Local verification"));
    assert!(pay_doc.unwrap().contains("Step C: Remote charge"));
    // base.InitContext -> has_receiver: false
    assert!(cs_facts
        .calls
        .iter()
        .any(|c| c.0 == "PayAsync" && c.1 == "InitContext" && !c.2));
    // this.VerifyRequest -> has_receiver: false
    assert!(cs_facts
        .calls
        .iter()
        .any(|c| c.0 == "PayAsync" && c.1 == "VerifyRequest" && !c.2));
    // gateway.ChargeAsync -> has_receiver: true
    assert!(cs_facts
        .calls
        .iter()
        .any(|c| c.0 == "PayAsync" && c.1 == "ChargeAsync" && c.2));

    // 5. Ruby: Module, Class, Def, self.method calls
    let rb = r#"
# Module documentation
module Billing
  # Controller documentation
  class InvoiceController
    # Issues an invoice to the client
    def issue_invoice(order)
      # Step 1: local calculation
      self.calc_tax(order)
      # Step 2: notify billing gateway
      Stripe.charge(order)
      # Step 3: log event
      logger.info("issued")
    end

    def calc_tax(order)
      round(order.total * 0.2)
    end
  end
end
"#;
    let rb_facts = Language::Ruby.parse(rb);
    assert_eq!(
        rb_facts.defines,
        vec!["Billing", "InvoiceController", "issue_invoice", "calc_tax"]
    );
    let inv_doc = rb_facts
        .docs
        .iter()
        .find(|(n, _)| n == "issue_invoice")
        .map(|(_, d)| d.as_str());
    assert!(inv_doc.unwrap().contains("Issues an invoice"));
    assert!(inv_doc.unwrap().contains("Step 1: local calculation"));
    assert!(inv_doc.unwrap().contains("Step 2: notify billing gateway"));
    // self.calc_tax -> has_receiver: false
    assert!(rb_facts
        .calls
        .iter()
        .any(|c| c.0 == "issue_invoice" && c.1 == "calc_tax" && !c.2));
    // Stripe.charge -> has_receiver: true
    assert!(rb_facts
        .calls
        .iter()
        .any(|c| c.0 == "issue_invoice" && c.1 == "charge" && c.2));

    // 6. PHP: Namespace, Class, Trait, $this-> / self:: calls
    let php = r#"
<?php
namespace App\Services;

/**
 * Authentication management
 */
class AuthService {
    /**
     * Authenticates user credentials
     */
    public function login(string $user, string $pass): bool {
        // Step 1: verify hash locally
        $ok = $this->verifyPassword($pass);
        // Step 2: check static cache
        $cached = self::checkCache($user);
        // Step 3: token generation
        return $this->jwt->createToken($user);
    }

    private function verifyPassword(string $pass): bool {
        return true;
    }

    public static function checkCache(string $user): bool {
        return false;
    }
}
"#;
    let php_facts = Language::Php.parse(php);
    assert_eq!(
        php_facts.defines,
        vec![
            "App\\Services",
            "AuthService",
            "login",
            "verifyPassword",
            "checkCache"
        ]
    );
    let login_doc = php_facts
        .docs
        .iter()
        .find(|(n, _)| n == "login")
        .map(|(_, d)| d.as_str());
    assert!(login_doc
        .unwrap()
        .contains("Authenticates user credentials"));
    assert!(login_doc.unwrap().contains("Step 1: verify hash locally"));
    assert!(login_doc.unwrap().contains("Step 2: check static cache"));
    // $this->verifyPassword -> has_receiver: false
    assert!(php_facts
        .calls
        .iter()
        .any(|c| c.0 == "login" && c.1 == "verifyPassword" && !c.2));
    // self::checkCache -> has_receiver: false
    assert!(php_facts
        .calls
        .iter()
        .any(|c| c.0 == "login" && c.1 == "checkCache" && !c.2));
    // $this->jwt->createToken -> has_receiver: true
    assert!(php_facts
        .calls
        .iter()
        .any(|c| c.0 == "login" && c.1 == "createToken" && c.2));

    // 7. Swift: Class, Actor, Protocol, self./super. calls, print() call
    let swift = r#"
/// User profile manager
@MainActor
public class UserManager {
    /// Loads user data from remote repository
    public func loadProfile(userId: String) async -> User {
        // Step 1: check local cache
        self.checkCache(userId)
        // Step 2: audit log
        super.recordAccess(userId)
        // Step 3: fetch remote data
        let u = await apiClient.fetchUser(userId)
        print("Loaded user profile")
        return u
    }

    func checkCache(_ id: String) {
        localLookup(id)
    }
}
"#;
    let swift_facts = Language::Swift.parse(swift);
    assert_eq!(
        swift_facts.defines,
        vec!["UserManager", "loadProfile", "checkCache"]
    );
    let lp_doc = swift_facts
        .docs
        .iter()
        .find(|(n, _)| n == "loadProfile")
        .map(|(_, d)| d.as_str());
    assert!(lp_doc
        .unwrap()
        .contains("Loads user data from remote repository"));
    assert!(lp_doc.unwrap().contains("Step 1: check local cache"));
    assert!(lp_doc.unwrap().contains("Step 2: audit log"));
    // self.checkCache -> has_receiver: false
    assert!(swift_facts
        .calls
        .iter()
        .any(|c| c.0 == "loadProfile" && c.1 == "checkCache" && !c.2));
    // super.recordAccess -> has_receiver: false
    assert!(swift_facts
        .calls
        .iter()
        .any(|c| c.0 == "loadProfile" && c.1 == "recordAccess" && !c.2));
    // apiClient.fetchUser -> has_receiver: true
    assert!(swift_facts
        .calls
        .iter()
        .any(|c| c.0 == "loadProfile" && c.1 == "fetchUser" && c.2));
    // print("Loaded user profile") -> has_receiver: false
    assert!(swift_facts
        .calls
        .iter()
        .any(|c| c.0 == "loadProfile" && c.1 == "print" && !c.2));

    // 8. Scala: Package, Object, Class, Def, Val, type, Calls
    let scala = r#"
package com.example.analytics

/**
 * Metrics calculation engine
 */
object MetricsEngine {
    val Version = "2.0"
    type Score = Double

    /**
     * Computes aggregate score
     */
    def computeScore(data: List[Double]): Score = {
        // Step 1: normalize dataset
        val norm = this.normalize(data)
        // Step 2: compute mean via math library
        val avg = Math.sqrt(data.sum)
        println("Score computed")
        avg
    }

    def normalize(data: List[Double]): List[Double] = {
        data
    }
}
"#;
    let scala_facts = Language::Scala.parse(scala);
    assert_eq!(
        scala_facts.defines,
        vec![
            "com.example.analytics",
            "MetricsEngine",
            "Version",
            "Score",
            "computeScore",
            "normalize"
        ]
    );
    let cs_doc = scala_facts
        .docs
        .iter()
        .find(|(n, _)| n == "computeScore")
        .map(|(_, d)| d.as_str());
    assert!(cs_doc.unwrap().contains("Computes aggregate score"));
    assert!(cs_doc.unwrap().contains("Step 1: normalize dataset"));
    assert!(cs_doc
        .unwrap()
        .contains("Step 2: compute mean via math library"));
    // this.normalize -> has_receiver: false
    assert!(scala_facts
        .calls
        .iter()
        .any(|c| c.0 == "computeScore" && c.1 == "normalize" && !c.2));
    // Math.sqrt -> has_receiver: true
    assert!(scala_facts
        .calls
        .iter()
        .any(|c| c.0 == "computeScore" && c.1 == "sqrt" && c.2));
    // println -> has_receiver: false
    assert!(scala_facts
        .calls
        .iter()
        .any(|c| c.0 == "computeScore" && c.1 == "println" && !c.2));

    // 9. Kotlin: Package, Class, Companion Object, Fun, Val, Typealias, Calls
    let kt = r#"
package com.example.data

typealias DataId = String

/**
 * Data processor service
 */
class DataProcessor {
    val timeout = 5000

    /**
     * Executes data processing pipeline
     */
    fun process(id: DataId): Boolean {
        // Step A: validate ID
        this.validate(id)
        // Step B: super log
        super.logStart(id)
        // Step C: database update
        db.save(id)
        println("Finished processing")
        return true
    }

    fun validate(id: DataId) {
        checkNotEmpty(id)
    }

    companion object Factory {
        fun create(): DataProcessor = DataProcessor()
    }
}
"#;
    let kt_facts = Language::Kotlin.parse(kt);
    assert_eq!(
        kt_facts.defines,
        vec![
            "com.example.data",
            "DataId",
            "DataProcessor",
            "timeout",
            "process",
            "validate",
            "Factory",
            "create"
        ]
    );
    let proc_doc = kt_facts
        .docs
        .iter()
        .find(|(n, _)| n == "process")
        .map(|(_, d)| d.as_str());
    assert!(proc_doc
        .unwrap()
        .contains("Executes data processing pipeline"));
    assert!(proc_doc.unwrap().contains("Step A: validate ID"));
    assert!(proc_doc.unwrap().contains("Step B: super log"));
    assert!(proc_doc.unwrap().contains("Step C: database update"));
    // this.validate -> has_receiver: false
    assert!(kt_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "validate" && !c.2));
    // super.logStart -> has_receiver: false
    assert!(kt_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "logStart" && !c.2));
    // db.save -> has_receiver: true
    assert!(kt_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "save" && c.2));
    // println -> has_receiver: false
    assert!(kt_facts
        .calls
        .iter()
        .any(|c| c.0 == "process" && c.1 == "println" && !c.2));

    // 10. Dart: Class, Mixin, Extension, Methods, Calls, Cascades
    let dart = r#"
library core.widgets;

/// Button widget implementation
class CustomButton {
    /// Renders button UI
    void render() {
        // Step 1: local theme setup
        this.applyTheme();
        // Step 2: super lifecycle
        super.initState();
        // Step 3: remote analytics
        analytics.trackEvent("render");
        print("rendered");
    }

    void applyTheme() {
        setupColors();
    }
}
"#;
    let dart_facts = Language::Dart.parse(dart);
    assert_eq!(
        dart_facts.defines,
        vec!["core.widgets", "CustomButton", "render", "applyTheme"]
    );
    let render_doc = dart_facts
        .docs
        .iter()
        .find(|(n, _)| n == "render")
        .map(|(_, d)| d.as_str());
    assert!(render_doc.unwrap().contains("Renders button UI"));
    assert!(render_doc.unwrap().contains("Step 1: local theme setup"));
    assert!(render_doc.unwrap().contains("Step 2: super lifecycle"));
    assert!(render_doc.unwrap().contains("Step 3: remote analytics"));
    // this.applyTheme -> has_receiver: false
    assert!(dart_facts
        .calls
        .iter()
        .any(|c| c.0 == "render" && c.1 == "applyTheme" && !c.2));
    // super.initState -> has_receiver: false
    assert!(dart_facts
        .calls
        .iter()
        .any(|c| c.0 == "render" && c.1 == "initState" && !c.2));
    // analytics.trackEvent -> has_receiver: true
    assert!(dart_facts
        .calls
        .iter()
        .any(|c| c.0 == "render" && c.1 == "trackEvent" && c.2));
    // print -> has_receiver: false
    assert!(dart_facts
        .calls
        .iter()
        .any(|c| c.0 == "render" && c.1 == "print" && !c.2));
}

#[test]
fn test_all_remaining_languages_full_suite() {
    // 1. Groovy
    let groovy = r#"
package com.demo
// Calculates discounts
class DiscountService {
    // Computes special rebate
    def calculateRebate(amount) {
        this.validate(amount)
        logger.info("rebate")
        return apply(amount)
    }
}
"#;
    let facts = Language::Groovy.parse(groovy);
    assert!(facts.defines.contains(&"DiscountService".to_string()));
    assert!(facts.defines.contains(&"calculateRebate".to_string()));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "calculateRebate" && c.1 == "validate" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "calculateRebate" && c.1 == "info" && c.2));

    // 2. VB.NET
    let vb = r#"
' Account module
Module AccountModule
    ''' Computes tax
    Public Function ComputeTax(amount As Double) As Double
        Me.Log("tax")
        Return Calculate(amount)
    End Function
End Module
"#;
    let facts = Language::Vb.parse(vb);
    assert!(facts.defines.contains(&"AccountModule".to_string()));
    assert!(facts.defines.contains(&"ComputeTax".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "ComputeTax" && d.contains("Computes tax")));

    // 3. COBOL
    let cobol = r#"
       IDENTIFICATION DIVISION.
       PROGRAM-ID. HELLO-WORLD.
       PROCEDURE DIVISION.
       MAIN-PROC.
           DISPLAY 'HELLO'.
           PERFORM CALC-SUB.
           STOP RUN.
       CALC-SUB.
           ADD 1 TO COUNTER.
"#;
    let facts = Language::Cobol.parse(cobol);
    assert!(facts.defines.contains(&"HELLO-WORLD".to_string()));
    assert!(facts.defines.contains(&"MAIN-PROC".to_string()));
    assert!(facts.defines.contains(&"CALC-SUB".to_string()));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "MAIN-PROC" && c.1 == "CALC-SUB"));

    // 4. Ada
    let ada = r#"
-- Package specification
package body Math_Utils is
   -- Computes factorial
   function Factorial(N : Integer) return Integer is
   begin
      Logger.Info(N);
      return Mult(N);
   end Factorial;
end Math_Utils;
"#;
    let facts = Language::Ada.parse(ada);
    assert!(facts.defines.contains(&"Math_Utils".to_string()));
    assert!(facts.defines.contains(&"Factorial".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "Factorial" && d.contains("Computes factorial")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "Factorial" && c.1 == "Info" && c.2));

    // 5. D
    let d = r#"
/// Math algorithms
module math.algo;
/// Computes square root
double calc_sqrt(double x) {
    validate(x);
    return sqrt(x);
}
"#;
    let facts = Language::D.parse(d);
    assert!(facts.defines.contains(&"math.algo".to_string()));
    assert!(facts.defines.contains(&"calc_sqrt".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "calc_sqrt" && d.contains("Computes square root")));

    // 6. WebAssembly Text (WAT)
    let wat = r#"
(module
  ;; Exported add function
  (func $add (param $x i32) (param $y i32) (result i32)
    (call $log (local.get $x))
    (i32.add (local.get $x) (local.get $y))
  )
)
"#;
    let facts = Language::Wat.parse(wat);
    assert!(facts.defines.contains(&"$add".to_string()));
    assert!(facts.calls.iter().any(|c| c.0 == "$add" && c.1 == "$log"));

    // 7. Erlang
    let erlang = r#"
-module(server).
-export([start/0, loop/1]).

%% Starts the server
start() ->
    init(),
    loop(0).

loop(State) ->
    handle_msg(State).
"#;
    let facts = Language::Erlang.parse(erlang);
    assert!(facts.defines.contains(&"server".to_string()));
    assert!(facts.defines.contains(&"start".to_string()));
    assert!(facts.defines.contains(&"loop".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "start" && d.contains("Starts the server")));
    assert!(facts.calls.iter().any(|c| c.0 == "start" && c.1 == "init"));

    // 8. F#
    let fsharp = r#"
namespace MathApp

/// Solves quadratic equation
module Solver =
    /// Computes roots
    let solve a b c =
        let d = calculate_discriminant a b c
        sqrt(d)
"#;
    let facts = Language::FSharp.parse(fsharp);
    assert!(facts.defines.contains(&"MathApp".to_string()));
    assert!(facts.defines.contains(&"Solver".to_string()));
    assert!(facts.defines.contains(&"solve".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "solve" && d.contains("Computes roots")));

    // 9. Clojure
    let clj = r#"
(ns my-project.core)

;; Calculates sum of items
(defn calculate-sum [items]
  (log/info items)
  (reduce + items))
"#;
    let facts = Language::Clojure.parse(clj);
    assert!(facts.defines.contains(&"my-project.core".to_string()));
    assert!(facts.defines.contains(&"calculate-sum".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "calculate-sum" && d.contains("Calculates sum")));

    // 10. Elm
    let elm = r#"
module Main exposing (update, view)

-- Updates model state
update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            calculateNewState model
"#;
    let facts = Language::Elm.parse(elm);
    assert!(facts.defines.contains(&"Main".to_string()));
    assert!(facts.defines.contains(&"update".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "update" && d.contains("Updates model state")));

    // 11. Gleam
    let gleam = r#"
import gleam/io

/// Main entry point
pub fn main() {
  io.println("Hello from Gleam!")
  process_data()
}
"#;
    let facts = Language::Gleam.parse(gleam);
    assert!(facts.defines.contains(&"main".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "main" && d.contains("Main entry point")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "main" && c.1 == "println" && c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "main" && c.1 == "process_data" && !c.2));

    // 12. PureScript
    let purs = r#"
module App.Main where

-- Computes fibonacci
fib :: Int -> Int
fib n =
  if n <= 1 then n else fib (n - 1) + fib (n - 2)
"#;
    let facts = Language::PureScript.parse(purs);
    assert!(facts.defines.contains(&"App.Main".to_string()));
    assert!(facts.defines.contains(&"fib".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "fib" && d.contains("Computes fibonacci")));

    // 13. Lisp / Scheme / Racket
    let lisp = r#"
;; Compute factorial
(defun factorial (n)
  (if (<= n 1)
      1
      (* n (factorial (- n 1)))))
"#;
    let facts = Language::Lisp.parse(lisp);
    assert!(facts.defines.contains(&"factorial".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "factorial" && d.contains("Compute factorial")));

    // 14. Lean
    let lean = r#"
/-- Proves theorem -/
theorem add_comm (n m : Nat) : n + m = m + n := by
  simp
"#;
    let facts = Language::Lean.parse(lean);
    assert!(facts.defines.contains(&"add_comm".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "add_comm" && d.contains("Proves theorem")));

    // 15. Nix
    let nix = r#"
{ pkgs ? import <nixpkgs> {} }:
let
  # Standard package configuration
  myApp = pkgs.stdenv.mkDerivation {
    pname = "myApp";
    buildInputs = [ pkgs.rustc ];
  };
in myApp
"#;
    let facts = Language::Nix.parse(nix);
    assert!(facts.defines.contains(&"myApp".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "myApp" && d.contains("Standard package configuration")));

    // 16. JSON
    let json = r#"
{
  "name": "my-package",
  "version": "1.0.0",
  "scripts": {
    "build": "tsc"
  }
}
"#;
    let facts = Language::Json.parse(json);
    assert!(facts.defines.contains(&"name".to_string()));
    assert!(facts.defines.contains(&"scripts".to_string()));

    // 17. Vue SFC
    let vue = r#"
<template>
  <button @click="handleClick">{{ title }}</button>
</template>

<script>
// User profile component
export default {
  methods: {
    handleClick() {
      this.track();
      api.send();
    }
  }
}
</script>
"#;
    let facts = Language::Vue.parse(vue);
    assert!(facts.defines.contains(&"handleClick".to_string()));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handleClick" && c.1 == "track" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handleClick" && c.1 == "send" && c.2));

    // 18. Svelte
    let svelte = r#"
<script>
  // Handles click event
  function handleClick() {
    count += 1;
    notify();
  }
</script>

<button on:click={handleClick}>Count</button>
"#;
    let facts = Language::Svelte.parse(svelte);
    assert!(facts.defines.contains(&"handleClick".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "handleClick" && d.contains("Handles click event")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handleClick" && c.1 == "notify"));

    // 19. XML / SVG / XAML
    let xml = r#"
<?xml version="1.0"?>
<!-- User entity definition -->
<User id="u123" name="Alice">
  <Role>Admin</Role>
</User>
"#;
    let facts = Language::Xml.parse(xml);
    assert!(facts.defines.contains(&"User".to_string()));
    assert!(facts.defines.contains(&"Role".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "User" && d.contains("User entity definition")));

    // 20. GraphQL
    let gql = r#"
"""
User entity representing a registered customer
"""
type User {
  id: ID!
  name: String!
}

type Query {
  getUser(id: ID!): User
}
"#;
    let facts = Language::GraphQL.parse(gql);
    assert!(facts.defines.contains(&"User".to_string()));
    assert!(facts.defines.contains(&"Query".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "User" && d.contains("User entity representing")));

    // 21. Thrift
    let thrift = r#"
// User authentication service
service AuthService {
    // Authenticates user credentials
    bool authenticate(1: string username, 2: string password)
}
"#;
    let facts = Language::Thrift.parse(thrift);
    assert!(facts.defines.contains(&"AuthService".to_string()));
    assert!(facts.defines.contains(&"authenticate".to_string()));

    // 22. FlatBuffers
    let fbs = r#"
namespace Game.Sample;

// Monster table definition
table Monster {
  hp:short = 100;
  mana:short = 150;
  name:string;
}
"#;
    let facts = Language::FlatBuffers.parse(fbs);
    assert!(facts.defines.contains(&"Game.Sample".to_string()));
    assert!(facts.defines.contains(&"Monster".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "Monster" && d.contains("Monster table definition")));

    // 23. Cap'n Proto
    let capnp = r#"
@0xdbb9ad1f14bf0b36;

# Person structure
struct Person {
  name @0 :Text;
  email @1 :Text;
}
"#;
    let facts = Language::CapnProto.parse(capnp);
    assert!(facts.defines.contains(&"Person".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "Person" && d.contains("Person structure")));

    // 24. Cypher (Neo4j)
    let cypher = r#"
// Create user node
CREATE (u:User {name: 'Alice'})
MATCH (u:User)
WHERE u.name = 'Alice'
RETURN u;
"#;
    let facts = Language::Cypher.parse(cypher);
    assert!(facts.defines.contains(&"User".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "User" && d.contains("Create user node")));

    // 25. PowerShell
    let ps = r#"
<#
.SYNOPSIS
Deploys the service to cluster
#>
function Deploy-Service {
    [CmdletBinding()]
    param([string]$Env)
    
    $this.Validate($Env)
    $cluster.Deploy($Env)
    Write-Host "Done"
}
"#;
    let facts = Language::PowerShell.parse(ps);
    assert!(facts.defines.contains(&"Deploy-Service".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "Deploy-Service" && d.contains("Deploys the service")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "Deploy-Service" && c.1 == "Validate" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "Deploy-Service" && c.1 == "Deploy" && c.2));

    // 26. GDScript (Godot)
    let gd = r#"
extends Node2D

## Handles player movement
func move_player(delta):
    self.check_collision()
    audio.play_sound()
"#;
    let facts = Language::GdScript.parse(gd);
    assert!(facts.defines.contains(&"move_player".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "move_player" && d.contains("Handles player movement")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "move_player" && c.1 == "check_collision" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "move_player" && c.1 == "play_sound" && c.2));

    // 27. Batch
    let bat = r#"
:: Setup environment
:SETUP
echo "Setting up..."
call :BUILD_STEP
goto :EOF

:BUILD_STEP
echo "Building..."
"#;
    let facts = Language::Batch.parse(bat);
    assert!(facts.defines.contains(&"SETUP".to_string()));
    assert!(facts.defines.contains(&"BUILD_STEP".to_string()));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "SETUP" && c.1 == "BUILD_STEP"));

    // 28. Fish Shell
    let fish = r#"
# Greps and formats log entries
function format_logs
    parse_entry $argv
    logger.emit $argv
end
"#;
    let facts = Language::Fish.parse(fish);
    assert!(facts.defines.contains(&"format_logs".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "format_logs" && d.contains("Greps and formats")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "format_logs" && c.1 == "parse_entry" && !c.2));

    // 29. MATLAB
    let matlab = r#"
%% Calculates vector magnitude
function mag = calc_magnitude(vec)
    % Inner body step
    validate_vector(vec);
    mag = norm(vec);
end
"#;
    let facts = Language::Matlab.parse(matlab);
    assert!(facts.defines.contains(&"calc_magnitude".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "calc_magnitude" && d.contains("Calculates vector magnitude")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "calc_magnitude" && c.1 == "validate_vector"));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "calc_magnitude" && c.1 == "norm"));

    // 30. Mojo
    let mojo = r#"
# Fast matrix multiplier
struct MatrixMultiplier:
    # Performs multiplication
    fn multiply(self, a: Tensor, b: Tensor) -> Tensor:
        self.check_dims(a, b)
        return compute_gemm(a, b)
"#;
    let facts = Language::Mojo.parse(mojo);
    assert!(facts.defines.contains(&"MatrixMultiplier".to_string()));
    assert!(facts.defines.contains(&"multiply".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "multiply" && d.contains("Performs multiplication")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "multiply" && c.1 == "check_dims" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "multiply" && c.1 == "compute_gemm" && !c.2));

    // 31. Fortran
    let fortran = r#"
! Module for mathematical solvers
module SolverModule
contains
    ! Solves equation
    subroutine SolveSystem(A, B)
        call ValidateMatrix(A)
        call Factorize(A)
    end subroutine SolveSystem
end module SolverModule
"#;
    let facts = Language::Fortran.parse(fortran);
    assert!(facts.defines.contains(&"SolverModule".to_string()));
    assert!(facts.defines.contains(&"SolveSystem".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "SolveSystem" && d.contains("Solves equation")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "SolveSystem" && c.1 == "ValidateMatrix"));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "SolveSystem" && c.1 == "Factorize"));

    // 32. VHDL
    let vhdl = r#"
-- Counter entity
entity Counter is
    port (clk : in bit; rst : in bit; count : out integer);
end Counter;

-- Architecture of counter
architecture Behavioral of Counter is
    -- Procedure to increment
    procedure Increment(val : inout integer) is
    begin
        Validate(val);
    end procedure Increment;
begin
end Behavioral;
"#;
    let facts = Language::Vhdl.parse(vhdl);
    assert!(facts.defines.contains(&"Counter".to_string()));
    assert!(facts.defines.contains(&"Behavioral".to_string()));
    assert!(facts.defines.contains(&"Increment".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "Counter" && d.contains("Counter entity")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "Increment" && c.1 == "Validate"));

    // 33. Shader (WGSL & GLSL/HLSL)
    let wgsl = r#"
// Vertex output structure
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

// Computes vertex transform
@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = transform_pos(model.pos);
    return out;
}
"#;
    let facts = Language::Shader.parse(wgsl);
    assert!(facts.defines.contains(&"VertexOutput".to_string()));
    assert!(facts.defines.contains(&"vs_main".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "vs_main" && d.contains("Computes vertex transform")));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "vs_main" && c.1 == "transform_pos"));

    // 34. LaTeX
    let tex = r#"
% Global document theorem environment
\newcommand{\myTheorem}[2]{\textbf{#1}: #2}

% Main abstract
\section{Abstract}
"#;
    let facts = Language::Latex.parse(tex);
    assert!(facts.defines.contains(&"myTheorem".to_string()));
    assert!(facts.defines.contains(&"section:Abstract".to_string()));
    assert!(facts
        .docs
        .iter()
        .any(|(n, d)| n == "myTheorem" && d.contains("Global document theorem environment")));
}

#[test]
fn test_kotlin_scala_dart_field_access_and_type_isolation() {
    // 1. Kotlin: Return type isolation and zero duplicate calls
    let kotlin_code = r#"
fun handle(req: Request): Response {
    val body = parse(req)
    val total = order.total
    val result = order.pay(body)
    return respond(result)
}
"#;
    let facts = Language::Kotlin.parse(kotlin_code);
    assert_eq!(facts.defines, vec!["handle"]);
    // Must NOT contain type names as calls
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Response"),
        "Response must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Request"),
        "Request must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "body"),
        "body must not be in calls: {:?}",
        facts.calls
    );
    // Field access order.total must not be in calls
    assert!(
        !facts.calls.iter().any(|c| c.1 == "total"),
        "order.total must not be in calls: {:?}",
        facts.calls
    );
    // Real call edges
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "parse" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "pay" && c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "respond" && !c.2));
    // Exact 3 calls, no duplicates
    assert_eq!(
        facts.calls.len(),
        3,
        "Expected exactly 3 calls, got: {:?}",
        facts.calls
    );

    // 2. Scala: Return type isolation and field access vs method call
    let scala_code = r#"
def handle(req: Request): Response = {
    val body = parse(req)
    val total = order.total
    val result = order.pay(body)
    respond(result)
}
"#;
    let facts = Language::Scala.parse(scala_code);
    assert_eq!(facts.defines, vec!["handle"]);
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Response"),
        "Response must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Request"),
        "Request must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "total"),
        "order.total must not be in calls: {:?}",
        facts.calls
    );
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "parse" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "pay" && c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "respond" && !c.2));
    assert_eq!(
        facts.calls.len(),
        3,
        "Expected exactly 3 calls, got: {:?}",
        facts.calls
    );

    // 3. Dart: Return type isolation and field access vs method call
    let dart_code = r#"
Response handle(Request req) {
    var body = parse(req);
    var total = order.total;
    var result = order.pay(body);
    return respond(result);
}
"#;
    let facts = Language::Dart.parse(dart_code);
    assert_eq!(facts.defines, vec!["handle"]);
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Response"),
        "Response must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "Request"),
        "Request must not be in calls: {:?}",
        facts.calls
    );
    assert!(
        !facts.calls.iter().any(|c| c.1 == "total"),
        "order.total must not be in calls: {:?}",
        facts.calls
    );
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "parse" && !c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "pay" && c.2));
    assert!(facts
        .calls
        .iter()
        .any(|c| c.0 == "handle" && c.1 == "respond" && !c.2));
    assert_eq!(
        facts.calls.len(),
        3,
        "Expected exactly 3 calls, got: {:?}",
        facts.calls
    );
}

#[test]
fn java_anonymous_class_is_not_a_definition() {
    // Through the rule route, which is the scanner `glasir` runs.
    let facts = native_parsers::rules::active().parse(
        Language::Java,
        "class Fixtures {\n    List<PetType> make() {\n        list.add(new PetType() {\n            public String getName() { return \"cat\"; }\n        });\n        return list;\n    }\n}\n",
    );
    assert_eq!(facts.defines, vec!["Fixtures", "make", "getName"]);
    assert!(facts.calls.iter().any(|c| c.0 == "make" && c.1 == "PetType"));
}

#[test]
fn gdscript_static_func_after_an_inner_class_is_top_level() {
    // `func` sits at column 7 in `static func`, which read as nested in `Meta`.
    let facts = native_parsers::rules::active().parse(
        Language::GdScript,
        "class Meta:\n\tvar uniforms = []\n\n\nstatic func build(\n\tshader\n) -> void:\n\tHelper.make(shader)\n",
    );
    assert!(
        facts.calls.iter().any(|c| c.0 == "build" && c.1 == "make"),
        "{:?}",
        facts.calls
    );
}

#[test]
fn an_indented_body_ends_for_attribution_too() {
    // The range closed on the dedent while the caller stack kept the
    // function: every top-level line after a `def` was attributed to it.
    let rules = native_parsers::rules::active();
    let py = rules.parse(
        Language::Python,
        "def outer(items):\n    def inner(h):\n        return h\n\n    for i in items:\n        load(i)\n\ndef other():\n    pass\n\nfor x in data:\n    store(x)\n",
    );
    for (facts, caller, callee) in [
        (&py, "outer", "load"),
        (&py, "<module>", "store"),
    ] {
        assert!(
            facts.calls.iter().any(|c| c.0 == caller && c.1 == callee),
            "{caller} -> {callee}: {:?}",
            facts.calls
        );
    }
}
