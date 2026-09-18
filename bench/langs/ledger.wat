;; Keeps the running balance for one account.
(module
  (global $LIMIT i32 (i32.const 5000))

  (func $refuse (param $owner i32) (result i32)
    (call $warn_owner (local.get $owner))
    (i32.const 0))

  (func $commit_entry (param $owner i32) (param $amount i32) (result i32)
    (call $write_entry (local.get $owner) (local.get $amount))
    (local.get $amount))

  ;; Bills the account and returns what is left.
  (func $charge (param $owner i32) (param $amount i32) (result i32)
    (call $refuse (local.get $owner))
    (call $commit_entry (local.get $owner) (local.get $amount)))

  ;; A function may name itself through its export instead of a `$name`, which
  ;; is what real modules written for a JavaScript host do. The fixture carried
  ;; only the `$name` form, so the corpus is what found it: every such function
  ;; was anonymous and its calls fell to `<module>`.
  (func (export "warn_owner") (param $owner i32) (result i32)
    (call $charge (local.get $owner) (i32.const 1))))
