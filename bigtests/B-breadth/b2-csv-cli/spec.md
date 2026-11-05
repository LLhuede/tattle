# csvtool: a command-line CSV toolkit

Build a command-line tool called `csvtool` with Node.js 18 (plain JS or TypeScript, your choice; no runtime dependencies are needed). `package.json` must have a `bin` entry for `csvtool` pointing at the entry file (if you use TypeScript, `npm run build` must produce it). The tool is run as `node <that file> <subcommand> [options] [file]`.

## Input
- The CSV comes from the `file` argument, or from standard input when no file is given (or the file is `-`).
- The first row is the header. Header names are the column names (case-sensitive, exact match).
- Format is RFC 4180 style: comma separator; fields may be wrapped in double quotes; inside quotes a comma, a line break or a doubled `""` (meaning one `"`) is data. Lines end with `\n` or `\r\n`. A final line break is optional. Completely blank lines are ignored.
- Every data row must have the same number of fields as the header, otherwise it's an error.
- Input with no rows at all (not even a header) is a data error (see exit codes).
- All values are text. Numeric behaviour is described per subcommand.

## Options
- Options can appear before or after the file argument, and a value can be given as `--name value` or `--name=value`.
- `--format csv|json` (all subcommands, default `csv`) chooses the output:
  - `csv`: a header line, then one line per row, with `\n` line endings and a final `\n`. A field is wrapped in double quotes (with `"` doubled) when it contains a comma, a double quote or a line break; other fields are written as they are. A result with no rows still prints the header line.
  - `json`: one JSON array of objects (keys in column order), followed by a `\n`. Values taken from the input are strings. An empty result is `[]`.
- `--help` (or `-h`) prints usage that names every subcommand to standard output and exits 0.

## Subcommands

### `select --columns a,b,c`
Outputs only the named columns, in the order given by `--columns`. A column can be named twice; it then appears twice in CSV output (and in JSON the later one wins, since keys are unique). `--columns` is required.

### `filter --where <condition> [--where <condition> …]`
Keeps the rows for which every condition is true. A condition is `<column><operator><value>`, written as one argument, for example `age>=30` or `name=Alice`. Operators: `=`, `!=`, `>`, `>=`, `<`, `<=` and `~` (the cell contains the value as a substring, case-sensitive). When looking for the operator, the two-character ones (`>=`, `<=`, `!=`) are checked before the one-character ones, and the first operator found, scanning left to right, splits the condition. The value is everything after the operator (it may be empty).
- For `>`, `>=`, `<`, `<=`, `=` and `!=`: if both the cell and the value are numbers (a number is text that fully matches an optional minus sign, digits and an optional fractional part, like `-3`, `12`, `0.5`), they are compared as numbers (so `10>9` is true and `1.0=1` is true). Otherwise they are compared as text (character by character, case-sensitive).
- At least one `--where` is required. Row order is kept. The output has all columns.

### `sort --by <column> [--desc]`
Sorts all rows by that column, ascending unless `--desc` is given. If every non-empty value in the column is a number (as defined above), the sort is numeric, otherwise it is by text (plain character comparison, so `B` sorts before `a`). The sort is stable: rows that compare equal keep their input order (also with `--desc`). Rows where the cell is empty go last in both directions. `--by` is required.

### `aggregate --fn <fn> [--column <column>] [--group-by <column>]`
`--fn` is one of `count`, `sum`, `avg`, `min`, `max`.
- `count` counts rows (`--column` is not needed and is ignored). The other functions need `--column`, and every value in it must be a number (empty values are skipped), otherwise it's an error that names the column and the offending value.
- Without `--group-by` the output has exactly one row. With `--group-by g`, one row per distinct value of `g`, in order of first appearance, with `g` as the first column.
- The result column is named `count` for `count` and `<fn>_<column>` for the others (for example `sum_amount`). Result values are plain JavaScript numbers printed in their usual form (so `avg` of 1 and 2 is `1.5`); in JSON they are numbers, not strings. `sum`/`min`/`max`/`avg` over a group with no numeric values give an empty value in CSV and `null` in JSON.
- `avg` is the sum divided by the number of non-empty values.

## Errors and exit codes
- Exit code `0` on success.
- Exit code `2` for usage errors: no subcommand, an unknown subcommand, a missing required option, an unknown option, an invalid `--fn`/`--format` value, or a column that isn't in the header (the message names the column).
- Exit code `1` for problems with the data or the input: a file that can't be read (the message includes the file name), a CSV with an unterminated quote, input with no header row, a row with the wrong number of fields (the message says `row N`, where the header is row 1), or a non-numeric value in an aggregate.
- On any error: nothing is written to standard output, and a line starting with `error: ` and a clear message goes to standard error (more lines, such as the usage, may follow).
- When there is no subcommand, print the usage to standard error and exit with code 2.
