-- A minimal pure-Lua JSON encoder/decoder for the livtet plugin stdlib.
-- Based on dkjson by David Kolf (MIT licensed).

local dkjson = {}

local function encode_value(v)
    local t = type(v)
    if t == "nil" then
        return "null"
    elseif t == "boolean" then
        return v and "true" or "false"
    elseif t == "number" then
        if v ~= v then
            return "null"
        end
        return string.format("%.17g", v)
    elseif t == "string" then
        return '"'
            .. v:gsub('[%c\\"]', {
                ["\b"] = "\\b",
                ["\f"] = "\\f",
                ["\n"] = "\\n",
                ["\r"] = "\\r",
                ["\t"] = "\\t",
                ["\\"] = "\\\\",
                ['"'] = '\\"',
            })
            .. '"'
    elseif t == "table" then
        local arr = {}
        local is_array = true
        local max = 0
        for k, _ in pairs(v) do
            if type(k) ~= "number" or k < 1 or math.floor(k) ~= k then
                is_array = false
                break
            end
            if k > max then
                max = k
            end
        end
        if is_array and next(v) ~= nil then
            for i = 1, max do
                table.insert(arr, encode_value(v[i]))
            end
            return "[" .. table.concat(arr, ",") .. "]"
        elseif is_array and next(v) == nil then
            return "[]"
        else
            for k, val in pairs(v) do
                table.insert(
                    arr,
                    encode_value(tostring(k)) .. ":" .. encode_value(val)
                )
            end
            return "{" .. table.concat(arr, ",") .. "}"
        end
    else
        error("cannot encode " .. t)
    end
end

function dkjson.encode(v)
    return encode_value(v)
end

-- Minimal JSON decoder

local parse_string, parse_number, parse_object, parse_array

local function skip_whitespace(s, pos)
    while pos <= #s do
        local c = s:sub(pos, pos)
        if c ~= " " and c ~= "\t" and c ~= "\n" and c ~= "\r" then
            return pos
        end
        pos = pos + 1
    end
    return pos
end

local function parse_value(s, pos)
    pos = skip_whitespace(s, pos)
    if pos > #s then
        error("unexpected end of JSON")
    end
    local c = s:sub(pos, pos)
    if c == '"' then
        return parse_string(s, pos)
    elseif c == "{" then
        return parse_object(s, pos)
    elseif c == "[" then
        return parse_array(s, pos)
    elseif c == "t" then
        if s:sub(pos, pos + 3) == "true" then
            return true, pos + 4
        end
    elseif c == "f" then
        if s:sub(pos, pos + 4) == "false" then
            return false, pos + 5
        end
    elseif c == "n" then
        if s:sub(pos, pos + 3) == "null" then
            return dkjson.null, pos + 4
        end
    elseif c == "-" or (c >= "0" and c <= "9") then
        return parse_number(s, pos)
    end
    error("unexpected character '" .. c .. "' at position " .. pos)
end

function parse_string(s, pos)
    pos = pos + 1
    local result = {}
    while pos <= #s do
        local c = s:sub(pos, pos)
        if c == '"' then
            return table.concat(result), pos + 1
        end
        if c == "\\" then
            pos = pos + 1
            local esc = s:sub(pos, pos)
            if esc == "b" then
                table.insert(result, "\b")
            elseif esc == "f" then
                table.insert(result, "\f")
            elseif esc == "n" then
                table.insert(result, "\n")
            elseif esc == "r" then
                table.insert(result, "\r")
            elseif esc == "t" then
                table.insert(result, "\t")
            elseif esc == "u" then
                local digits = s:sub(pos + 1, pos + 4)
                for i = 1, 4 do
                    local d = digits:sub(i, i)
                    if d == "" or not d:match("[0-9a-fA-F]") then
                        error("invalid unicode escape")
                    end
                end
                local code = tonumber(digits, 16)
                if code < 0x80 then
                    table.insert(result, string.char(code))
                elseif code < 0x800 then
                    table.insert(result, string.char(0xC0 + code / 64))
                    table.insert(result, string.char(0x80 + code % 64))
                else
                    table.insert(
                        result,
                        string.char(0xE0 + math.floor(code / 4096))
                    )
                    table.insert(
                        result,
                        string.char(0x80 + math.floor((code % 4096) / 64))
                    )
                    table.insert(result, string.char(0x80 + code % 64))
                end
                pos = pos + 4
            else
                table.insert(result, esc)
            end
        else
            table.insert(result, c)
        end
        pos = pos + 1
    end
    error("unterminated string")
end

function parse_number(s, pos)
    local start = pos
    if s:sub(pos, pos) == "-" then
        pos = pos + 1
    end
    while pos <= #s and s:sub(pos, pos):match("[0-9]") do
        pos = pos + 1
    end
    if s:sub(pos, pos) == "." then
        pos = pos + 1
        while pos <= #s and s:sub(pos, pos):match("[0-9]") do
            pos = pos + 1
        end
    end
    if s:sub(pos, pos) == "e" or s:sub(pos, pos) == "E" then
        pos = pos + 1
        if s:sub(pos, pos) == "+" or s:sub(pos, pos) == "-" then
            pos = pos + 1
        end
        while pos <= #s and s:sub(pos, pos):match("[0-9]") do
            pos = pos + 1
        end
    end
    return tonumber(s:sub(start, pos - 1)), pos
end

function parse_object(s, pos)
    pos = pos + 1
    local obj = {}
    pos = skip_whitespace(s, pos)
    if s:sub(pos, pos) == "}" then
        return obj, pos + 1
    end
    while true do
        pos = skip_whitespace(s, pos)
        local key, val
        key, pos = parse_string(s, pos)
        pos = skip_whitespace(s, pos)
        if s:sub(pos, pos) ~= ":" then
            error("expected ':'")
        end
        pos = pos + 1
        val, pos = parse_value(s, pos)
        obj[key] = val
        pos = skip_whitespace(s, pos)
        if s:sub(pos, pos) == "}" then
            return obj, pos + 1
        end
        if s:sub(pos, pos) ~= "," then
            error("expected ',' or '}'")
        end
        pos = pos + 1
    end
end

function parse_array(s, pos)
    pos = pos + 1
    local arr = {}
    pos = skip_whitespace(s, pos)
    if s:sub(pos, pos) == "]" then
        return arr, pos + 1
    end
    local i = 1
    while true do
        local val
        val, pos = parse_value(s, pos)
        arr[i] = val
        i = i + 1
        pos = skip_whitespace(s, pos)
        if s:sub(pos, pos) == "]" then
            return arr, pos + 1
        end
        if s:sub(pos, pos) ~= "," then
            error("expected ',' or ']'")
        end
        pos = pos + 1
    end
end

dkjson.null = {}

function dkjson.decode(s)
    if type(s) ~= "string" then
        error("decode expects string, got " .. type(s))
    end
    local val, pos = parse_value(s, 1)
    pos = skip_whitespace(s, pos)
    if pos <= #s then
        error("trailing garbage at position " .. pos)
    end
    return val
end

return dkjson
