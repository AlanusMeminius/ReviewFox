; Repo-owned CMake highlights for tree-sitter-cmake 0.7.5.
; Capture names must match the Diff syntax palette (issue 02).
; No Neovim predicates (#lua-match?, @spell, @none).

[
  (line_comment)
  (bracket_comment)
] @comment

[
  (quoted_argument)
  (bracket_argument)
] @string

(escape_sequence) @escape

(variable_ref) @variable

((unquoted_argument) @constant
  (#match? @constant "^[A-Z@][A-Z0-9_]+$"))

(normal_command
  (identifier) @function)

[
  (if)
  (elseif)
  (else)
  (endif)
  (foreach)
  (endforeach)
  (while)
  (endwhile)
  (function)
  (endfunction)
  (macro)
  (endmacro)
] @keyword

; Same identifier node as @function above; listed later so the highlighter
; applies @keyword after @function and it is what the stack leaves on top.
(normal_command
  (identifier) @keyword
  (#match? @keyword "^[Rr][Ee][Tt][Uu][Rr][Nn]$"))

[
  "("
  ")"
] @punctuation.bracket
