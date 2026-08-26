/*
The C surface between Rust and Luau's analysis frontend.

Eight functions, byte offsets in both directions, and two callbacks that
hand require resolution and source loading to Rust. Strings that the shim
returns belong to the session and stay valid until the next call on the
same session; Rust copies what it keeps.
*/

#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LarvaeSession LarvaeSession;

/* Rust resolves a require spec from a module; returns a path the loader
   understands, or null when nothing resolves. The buffer belongs to Rust
   and lives until the next resolver call. */
typedef const char* (*larvae_resolve_fn)(void* userdata, const char* from, const char* spec);

/* Rust loads the text of a module path; null when the file is gone. */
typedef const char* (*larvae_load_fn)(void* userdata, const char* path);

LarvaeSession* larvae_session_new(void);
void larvae_session_free(LarvaeSession* s);

void larvae_set_resolver(LarvaeSession* s, void* userdata, larvae_resolve_fn resolve, larvae_load_fn load);

/* Declaration text in .d.luau form, loaded into the global scope. */
int larvae_set_definitions(LarvaeSession* s, const char* name, const char* source);

/* The text of one open module; replaces what the session held. */
void larvae_open(LarvaeSession* s, const char* path, const char* text);

/* Drop the cached state of one module and everything that depends on it. */
void larvae_invalidate(LarvaeSession* s, const char* path);

/* One diagnostic, byte addressed against the module's text. */
typedef struct {
    uint32_t start;
    uint32_t end;
    uint8_t severity; /* 1 error, 2 warning */
    const char* message;
} LarvaeDiag;

/* Type-check one module. Returns how many diagnostics, writes at most cap. */
size_t larvae_check(LarvaeSession* s, const char* path, LarvaeDiag* out, size_t cap);

/* The type at a byte offset, rendered; null when nothing is there. */
const char* larvae_hover(LarvaeSession* s, const char* path, uint32_t byte);

typedef struct {
    const char* label;
    uint8_t kind; /* CompletionItemKind of the protocol */
} LarvaeCompletion;

/* Completions at a byte offset. Returns how many, writes at most cap. */
size_t larvae_completions(LarvaeSession* s, const char* path, uint32_t byte, LarvaeCompletion* out, size_t cap);

/* Where a name is declared.

   Line and character, and not a byte offset, because the answer often names
   a module the caller has no text for. Those are the units the protocol
   wants anyway, so nothing converts on either side.

   `path` is the module the declaration sits in, and it belongs to the
   session until the next call. */
typedef struct {
    const char* path;
    uint32_t start_line;
    uint32_t start_character;
    uint32_t end_line;
    uint32_t end_character;
} LarvaeLocation;

/* The declaration of whatever sits at a byte offset. 1 on success. */
int larvae_definition(LarvaeSession* s, const char* path, uint32_t byte, LarvaeLocation* out);

/* The declaration of the TYPE of whatever sits at a byte offset. 1 on success. */
int larvae_type_definition(LarvaeSession* s, const char* path, uint32_t byte, LarvaeLocation* out);

#ifdef __cplusplus
}
#endif
