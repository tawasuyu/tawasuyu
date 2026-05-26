;; Plugin fixture: "hello-status".
;;
;; Lee el payload de args que el host escribió en memoria justo
;; después del nombre de la capability, y lo concatena con un saludo
;; fijo "hola, " en otro offset. Después emite el resultado via
;; `plugin.set_status`.
;;
;; Layout de memoria al entrar `_invoke`:
;;   [0 .. cap_len)               nombre de capability (UTF-8)
;;   [cap_len .. cap_len+arg_len) args del host (UTF-8)
;;
;; El plugin coloca su buffer de salida en el offset 256 para no
;; pisar lo anterior. v0 del ABI no negocia layouts — la convención
;; es que el plugin elige offsets altos.
(module
  (import "plugin" "log"        (func $log        (param i32 i32)))
  (import "plugin" "set_status" (func $set_status (param i32 i32)))

  (memory (export "memory") 1)

  ;; "hola, " en offset 256 (6 bytes)
  (data (i32.const 256) "hola, ")

  (func (export "_invoke")
        (param $cap_ptr i32) (param $cap_len i32)
        (param $arg_ptr i32) (param $arg_len i32)
        (result i32)
    ;; Traza para debug: el host capturará "[plugin] greet"
    (call $log (i32.const 256) (i32.const 5))

    ;; Copia los args al final del prefijo "hola, " en 256+6=262
    (memory.copy
      (i32.const 262)       ;; dst = 256 + len("hola, ")
      (local.get $arg_ptr)  ;; src = donde el host puso args
      (local.get $arg_len))

    ;; Total len = 6 ("hola, ") + arg_len
    (call $set_status
      (i32.const 256)
      (i32.add (i32.const 6) (local.get $arg_len)))

    (i32.const 0)
  )
)
