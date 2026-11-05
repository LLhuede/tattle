# logstat: an access-log analyser CLI

A command line tool for Node.js 18 that reads web server access logs and prints a summary: top paths, status code counts and response-time percentiles. No runtime dependencies are needed.

## Package
- `package.json` has `"name": "logstat"` and a `bin` entry named `logstat` (suggested file: `bin/logstat.js`). The tests run it as `node <that file> [args]`, so it must work without being installed globally and without a build step the tests don't run (`npm run build` is run first if it exists).
- `npm test` runs your own tests.

## Usage
```
logstat [options] [file ...]
```
- With no files, or with a file named `-`, it reads standard input. Several files are read in the order given and counted together as one stream.
- Options can be written `--top 5` or `--top=5`.

## Log lines
One request per line, in Apache/nginx **common** format:
```
127.0.0.1 - frank [10/Oct/2023:13:55:36 -0700] "GET /apache_pb.gif?size=2 HTTP/1.0" 200 2326
```
or **combined** format (common plus two quoted fields):
```
127.0.0.1 - frank [10/Oct/2023:13:55:36 -0700] "GET /a HTTP/1.1" 200 2326 "http://ref.example/" "Mozilla/5.0 (X11; Linux) Gecko/2010"
```
Rules:
- The fields are: host, ident, user (each a token without spaces, possibly `-`), the time in square brackets as `DD/Mon/YYYY:HH:MM:SS +ZZZZ` (English month abbreviations, a numeric UTC offset), the request in double quotes as `METHOD target HTTP/x.y`, a three-digit status, and the byte count (digits, or `-`). Quoted fields never contain a `"` character.
- A line may end with one extra field: **the response time in milliseconds**, a non-negative number with an optional decimal part (for example `12`, `0.5`, `120.75`), separated by a space. It can follow the byte count (common) or the user agent (combined). Lines without it are still counted everywhere except in the response-time statistics.
- The **path** of a request is its target without the query string (`/apache_pb.gif?size=2` becomes `/apache_pb.gif`). Methods do not matter for path counting: `GET /a` and `POST /a` both count towards `/a`.
- Blank lines are ignored. Any other line that does not match the format (including a request field that is not `METHOD target HTTP/x.y`, a status that is not three digits, or an invalid date) is **malformed**.
- Malformed lines are skipped, and they are counted. They are reported in the output, and are not an error unless `--strict` is given. Nothing is printed to stderr for them in normal mode.

## What it reports
- `requests`: the number of well-formed lines that pass the filters.
- `malformed`: the number of malformed lines (filters don't affect this).
- Top paths: the paths with the most requests, highest count first, ties broken by path in ascending byte order. `--top N` sets how many (N is a positive integer; default 10).
- Status codes: the count per exact status code, and the count per class (`2xx`, `4xx`, ...). Only codes that occurred are listed.
- Response times (ms), over the lines that have one: `count`, `min`, `max`, `mean` (rounded to 2 decimal places) and percentiles. Percentiles use the **nearest-rank** method: sort the values ascending, then pN is the value at rank `ceil(N/100 * count)` (1-based), so it is always one of the actual values. The default percentiles are 50, 90, 95 and 99. `--percentiles 50,99` replaces the list with a comma-separated list of integers between 1 and 100 (output keys `p50`, `p99`, ...).

## Output formats
**JSON** (`--json`): a single JSON object followed by a newline, in this shape:
```json
{
  "requests": 5,
  "malformed": 1,
  "top_paths": [{ "path": "/a", "count": 3 }, { "path": "/b", "count": 2 }],
  "status": { "200": 4, "404": 1 },
  "status_classes": { "2xx": 4, "4xx": 1 },
  "response_time_ms": { "count": 4, "min": 10, "max": 90, "mean": 42.5, "p50": 30, "p90": 90, "p95": 90, "p99": 90 }
}
```
`response_time_ms` is `null` when no counted line has a response time. With no input at all the output is `requests: 0`, `top_paths: []`, empty `status` and `status_classes`, and `response_time_ms: null`, and the exit code is 0.

**Text** (the default): lines like these (headings start in column 0, entries are indented by two spaces):
```
requests: 5
malformed: 1
top paths:
  3 /a
  2 /b
status codes:
  200 4
  404 1
response time (ms):
  count: 4
  min: 10
  max: 90
  mean: 42.5
  p50: 30
  p90: 90
  p95: 90
  p99: 90
```
Top paths are `<count> <path>`, status codes are `<code> <count>` in ascending numeric order, and the response-time lines are `<name>: <value>`. When there are no response times, the last section is the single line `response time (ms): n/a`.

## Filters
All filters can be combined (a line must satisfy all of them), and they apply to every statistic.
- `--since T` keeps lines whose time is at or after T (inclusive). `--until T` keeps lines before T (exclusive).
- `T` is an ISO 8601 timestamp with a `Z` or `+HH:MM`/`-HH:MM` offset (`2023-10-10T20:00:00Z`, seconds optional, fractional seconds allowed), or a plain date `YYYY-MM-DD`, meaning 00:00:00 UTC of that day. Times are compared as instants, so a line stamped `13:55:36 -0700` is the same moment as `20:55:36Z`.
- `--status S`: keep only lines with these statuses. `S` is a comma-separated list of exact codes or classes (`404`, `5xx`, `404,500,5xx`).
- `--method M`: keep only these request methods. A comma-separated list, case-insensitive (`--method get,post`).

## Errors and exit codes
Error messages go to stderr, start with `logstat: `, and nothing is written to stdout when the exit code is not 0.
- `0`: success (including when no line matched).
- `1`: an input file cannot be read (missing, a directory, no permission). The message names the file.
- `2`: a usage error: unknown option, an option missing its value, a value that is invalid (`--top` not a positive integer, a bad timestamp, `--since` later than `--until`, a bad `--status`, a bad `--percentiles`), or a malformed line with `--strict`.
- `--strict`: instead of skipping the first malformed line, stop with exit code 2 and the message `logstat: <source>:<line>: malformed log line`, where `<source>` is the file name as given (or `<stdin>`) and `<line>` is the 1-based line number in that source (blank lines count towards line numbers).
- `--help` / `-h`: print a usage text containing the word `Usage` to stdout and exit 0.
