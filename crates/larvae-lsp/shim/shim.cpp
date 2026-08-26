/*
The shim over Luau's analysis frontend.

One session owns one Frontend, a file resolver that calls back into Rust,
and the text of every open module. Positions convert here and nowhere
else: Luau speaks lines and columns, the boundary speaks bytes, and the
session holds the line index of each module it opened.

The strings the shim returns live in per-session buffers that the next
call on the same session overwrites. Rust copies what it keeps, and the
session is used from one thread, which the Rust side guarantees.
*/

#include "shim.h"

#include "Luau/AstQuery.h"
#include "Luau/Autocomplete.h"
#include "Luau/ConfigResolver.h"
#include "Luau/BuiltinDefinitions.h"
#include "Luau/Frontend.h"
#include "Luau/ToString.h"
#include "Luau/TypeAttach.h"

#include <map>
#include <optional>
#include <string>
#include <vector>

namespace
{

struct RustFileResolver : Luau::FileResolver
{
    void* userdata = nullptr;
    larvae_resolve_fn resolve = nullptr;
    larvae_load_fn load = nullptr;
    std::map<std::string, std::string>* open;

    std::optional<Luau::SourceCode> readSource(const Luau::ModuleName& name) override
    {
        auto it = open->find(name);
        if (it != open->end())
            return Luau::SourceCode{it->second, Luau::SourceCode::Module};

        if (load)
        {
            const char* text = load(userdata, name.c_str());
            if (text)
                return Luau::SourceCode{std::string(text), Luau::SourceCode::Module};
        }

        return std::nullopt;
    }

    std::optional<Luau::ModuleInfo> resolveModule(
        const Luau::ModuleInfo* context, Luau::AstExpr* node, const Luau::TypeCheckLimits&) override
    {
        if (!resolve || !context)
            return std::nullopt;

        auto* expr = node->as<Luau::AstExprConstantString>();
        if (!expr)
            return std::nullopt;

        std::string spec(expr->value.data, expr->value.size);
        const char* target = resolve(userdata, context->name.c_str(), spec.c_str());
        if (!target)
            return std::nullopt;

        return Luau::ModuleInfo{target};
    }
};

/* Byte offset <-> Luau Position, against one module's text. */
struct LineIndex
{
    std::vector<uint32_t> starts;

    explicit LineIndex(const std::string& text)
    {
        starts.push_back(0);
        for (uint32_t i = 0; i < text.size(); ++i)
            if (text[i] == '\n')
                starts.push_back(i + 1);
    }

    uint32_t byteOf(const Luau::Position& p, const std::string& text) const
    {
        uint32_t line = p.line < starts.size() ? starts[p.line] : (uint32_t)text.size();
        uint32_t byte = line + p.column;
        return byte > text.size() ? (uint32_t)text.size() : byte;
    }

    Luau::Position positionOf(uint32_t byte) const
    {
        uint32_t line = 0;
        for (size_t i = 0; i < starts.size(); ++i)
            if (starts[i] <= byte)
                line = (uint32_t)i;
            else
                break;
        return Luau::Position{line, byte - starts[line]};
    }
};

} // namespace

/*
The frontend keeps the type graphs it builds, or a hover has nothing to
find: the default discards a module's types the moment check returns.
*/
static Luau::FrontendOptions options()
{
    Luau::FrontendOptions o;
    o.retainFullTypeGraphs = true;
    return o;
}

struct LarvaeSession
{
    RustFileResolver files;
    Luau::NullConfigResolver configs;
    Luau::Frontend frontend;

    std::map<std::string, std::string> open;
    std::vector<std::string> diagStorage;
    std::vector<std::string> completionStorage;
    std::string hoverStorage;

    LarvaeSession()
        : frontend(&files, &configs, options())
    {
        files.open = &open;
        configs.defaultConfig.mode = Luau::Mode::Nonstrict;

        Luau::registerBuiltinGlobals(frontend, frontend.globals, false);
        Luau::registerBuiltinGlobals(frontend, frontend.globalsForAutocomplete, true);
        Luau::freeze(frontend.globals.globalTypes);
        Luau::freeze(frontend.globalsForAutocomplete.globalTypes);
    }
};

extern "C" {

LarvaeSession* larvae_session_new(void)
{
    return new LarvaeSession();
}

void larvae_session_free(LarvaeSession* s)
{
    delete s;
}

void larvae_set_resolver(LarvaeSession* s, void* userdata, larvae_resolve_fn resolve, larvae_load_fn load)
{
    s->files.userdata = userdata;
    s->files.resolve = resolve;
    s->files.load = load;
}

int larvae_set_definitions(LarvaeSession* s, const char* name, const char* source)
{
    Luau::unfreeze(s->frontend.globals.globalTypes);
    Luau::unfreeze(s->frontend.globalsForAutocomplete.globalTypes);

    Luau::LoadDefinitionFileResult result = s->frontend.loadDefinitionFile(
        s->frontend.globals, s->frontend.globals.globalScope, source, name, false, false);

    Luau::LoadDefinitionFileResult forAutocomplete = s->frontend.loadDefinitionFile(
        s->frontend.globalsForAutocomplete, s->frontend.globalsForAutocomplete.globalScope, source, name, false, true);

    Luau::freeze(s->frontend.globals.globalTypes);
    Luau::freeze(s->frontend.globalsForAutocomplete.globalTypes);

    return result.success && forAutocomplete.success ? 0 : 1;
}

void larvae_open(LarvaeSession* s, const char* path, const char* text)
{
    s->open[path] = text;
    s->frontend.markDirty(path);
}

void larvae_invalidate(LarvaeSession* s, const char* path)
{
    std::vector<Luau::ModuleName> dependents;
    s->frontend.markDirty(path, &dependents);

    for (const auto& name : dependents)
        s->frontend.markDirty(name);
}

size_t larvae_check(LarvaeSession* s, const char* path, LarvaeDiag* out, size_t cap)
{
    Luau::CheckResult result;

    try
    {
        result = s->frontend.check(path);
    }
    catch (const std::exception&)
    {
        return 0;
    }

    auto it = s->open.find(path);
    if (it == s->open.end())
        return 0;

    LineIndex lines(it->second);

    s->diagStorage.clear();
    // Reserved up front: a push past capacity moves the strings, and the
    // pointers already written into `out` would dangle.
    s->diagStorage.reserve(cap);
    size_t n = 0;

    for (const Luau::TypeError& error : result.errors)
    {
        if (error.moduleName != path)
            continue;

        if (n >= cap)
            break;

        s->diagStorage.push_back(Luau::toString(error));

        out[n].start = lines.byteOf(error.location.begin, it->second);
        out[n].end = lines.byteOf(error.location.end, it->second);
        out[n].severity = 1;
        out[n].message = s->diagStorage.back().c_str();
        ++n;
    }

    return n;
}

const char* larvae_hover(LarvaeSession* s, const char* path, uint32_t byte)
{
    auto it = s->open.find(path);
    if (it == s->open.end())
        return nullptr;

    try
    {
        s->frontend.check(path);
    }
    catch (const std::exception&)
    {
        return nullptr;
    }

    Luau::ModulePtr module = s->frontend.moduleResolver.getModule(path);
    const Luau::SourceModule* source = s->frontend.getSourceModule(path);
    if (!module || !source)
        return nullptr;

    LineIndex lines(it->second);
    Luau::Position position = lines.positionOf(byte);

    std::optional<Luau::TypeId> type = Luau::findTypeAtPosition(*module, *source, position);
    if (!type)
        return nullptr;

    Luau::ToStringOptions opts;
    opts.exhaustive = false;
    opts.maxTypeLength = 1000;

    s->hoverStorage = Luau::toString(*type, opts);
    return s->hoverStorage.c_str();
}

size_t larvae_completions(LarvaeSession* s, const char* path, uint32_t byte, LarvaeCompletion* out, size_t cap)
{
    auto it = s->open.find(path);
    if (it == s->open.end())
        return 0;

    LineIndex lines(it->second);
    Luau::Position position = lines.positionOf(byte);

    Luau::AutocompleteResult result;

    try
    {
        /*
        Autocomplete reads the module that the autocomplete typechecker
        built, which exists only after a check that asked for it.
        */
        Luau::FrontendOptions forAutocomplete = options();
        forAutocomplete.forAutocomplete = true;
        s->frontend.check(path, forAutocomplete);

        result = Luau::autocomplete(s->frontend, path, position,
            [](std::string, std::optional<const Luau::ExternType*>,
                std::optional<std::string>) -> std::optional<Luau::AutocompleteEntryMap> {
                return std::nullopt;
            });
    }
    catch (const std::exception&)
    {
        return 0;
    }

    s->completionStorage.clear();
    // Reserved for the same reason as the diagnostics: no reallocation
    // after the first pointer is handed out.
    s->completionStorage.reserve(cap);
    size_t n = 0;

    for (const auto& [label, entry] : result.entryMap)
    {
        if (n >= cap)
            break;

        s->completionStorage.push_back(label);

        uint8_t kind = 12; /* Value */
        switch (entry.kind)
        {
        case Luau::AutocompleteEntryKind::Property:
            kind = 5; /* Field */
            break;
        case Luau::AutocompleteEntryKind::Keyword:
            kind = 14; /* Keyword */
            break;
        case Luau::AutocompleteEntryKind::Module:
            kind = 9; /* Module */
            break;
        default:
            break;
        }

        if (entry.type && Luau::get<Luau::FunctionType>(Luau::follow(*entry.type)))
            kind = 3; /* Function */

        out[n].label = s->completionStorage.back().c_str();
        out[n].kind = kind;
        ++n;
    }

    return n;
}

} // extern "C"
