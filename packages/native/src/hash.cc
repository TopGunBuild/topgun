/**
 * Native xxHash64 implementation for TopGun
 *
 * Provides high-performance hashing for Merkle tree operations.
 * xxHash64 is 10-20x faster than SHA-256 for small inputs.
 */

#include <napi.h>

// xxHash header-only implementation
#define XXH_INLINE_ALL
#include "xxhash.h"

/**
 * Compute xxHash64 of a buffer.
 * @param buffer - Input data (Buffer or Uint8Array)
 * @param seed - Optional seed (default: 0)
 * @returns BigInt with 64-bit hash
 */
Napi::Value XxHash64(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    // Validate arguments
    if (info.Length() < 1) {
        Napi::TypeError::New(env, "Expected at least 1 argument")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    if (!info[0].IsBuffer() && !info[0].IsTypedArray()) {
        Napi::TypeError::New(env, "Expected Buffer or TypedArray")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    // Get buffer data
    uint8_t* data;
    size_t length;

    if (info[0].IsBuffer()) {
        auto buffer = info[0].As<Napi::Buffer<uint8_t>>();
        data = buffer.Data();
        length = buffer.Length();
    } else {
        auto typedArray = info[0].As<Napi::TypedArray>();
        data = static_cast<uint8_t*>(typedArray.ArrayBuffer().Data()) +
               typedArray.ByteOffset();
        length = typedArray.ByteLength();
    }

    // Get optional seed
    uint64_t seed = 0;
    if (info.Length() > 1 && info[1].IsBigInt()) {
        bool lossless;
        seed = info[1].As<Napi::BigInt>().Uint64Value(&lossless);
    } else if (info.Length() > 1 && info[1].IsNumber()) {
        seed = static_cast<uint64_t>(info[1].As<Napi::Number>().Int64Value());
    }

    // Compute hash
    XXH64_hash_t hash = XXH64(data, length, seed);

    // Return as BigInt for full 64-bit precision
    return Napi::BigInt::New(env, hash);
}

/**
 * Compute xxHash64 and return as 32-bit number (truncated).
 * Useful when BigInt overhead is not needed.
 */
Napi::Value XxHash64AsNumber(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (info.Length() < 1 || (!info[0].IsBuffer() && !info[0].IsTypedArray())) {
        Napi::TypeError::New(env, "Expected Buffer or TypedArray")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    uint8_t* data;
    size_t length;

    if (info[0].IsBuffer()) {
        auto buffer = info[0].As<Napi::Buffer<uint8_t>>();
        data = buffer.Data();
        length = buffer.Length();
    } else {
        auto typedArray = info[0].As<Napi::TypedArray>();
        data = static_cast<uint8_t*>(typedArray.ArrayBuffer().Data()) +
               typedArray.ByteOffset();
        length = typedArray.ByteLength();
    }

    uint64_t seed = 0;
    if (info.Length() > 1 && info[1].IsNumber()) {
        seed = static_cast<uint64_t>(info[1].As<Napi::Number>().Int64Value());
    }

    XXH64_hash_t hash = XXH64(data, length, seed);

    // Truncate to 32 bits for compatibility with current FNV-1a usage
    return Napi::Number::New(env, static_cast<uint32_t>(hash & 0xFFFFFFFF));
}

/**
 * Compute xxHash64 for multiple buffers in batch.
 * More efficient than calling XxHash64 repeatedly due to reduced JS<->C++ overhead.
 * @param buffers - Array of Buffers
 * @param seed - Optional seed
 * @returns Array of BigInt hashes
 */
Napi::Value XxHash64Batch(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!info[0].IsArray()) {
        Napi::TypeError::New(env, "Expected Array of Buffers")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    auto buffers = info[0].As<Napi::Array>();
    uint32_t count = buffers.Length();

    uint64_t seed = 0;
    if (info.Length() > 1 && info[1].IsBigInt()) {
        bool lossless;
        seed = info[1].As<Napi::BigInt>().Uint64Value(&lossless);
    } else if (info.Length() > 1 && info[1].IsNumber()) {
        seed = static_cast<uint64_t>(info[1].As<Napi::Number>().Int64Value());
    }

    Napi::Array results = Napi::Array::New(env, count);

    for (uint32_t i = 0; i < count; i++) {
        Napi::Value item = buffers.Get(i);

        if (!item.IsBuffer() && !item.IsTypedArray()) {
            results.Set(i, Napi::BigInt::New(env, static_cast<uint64_t>(0)));
            continue;
        }

        uint8_t* data;
        size_t length;

        if (item.IsBuffer()) {
            auto buffer = item.As<Napi::Buffer<uint8_t>>();
            data = buffer.Data();
            length = buffer.Length();
        } else {
            auto typedArray = item.As<Napi::TypedArray>();
            data = static_cast<uint8_t*>(typedArray.ArrayBuffer().Data()) +
                   typedArray.ByteOffset();
            length = typedArray.ByteLength();
        }

        XXH64_hash_t hash = XXH64(data, length, seed);
        results.Set(i, Napi::BigInt::New(env, hash));
    }

    return results;
}

/**
 * Batch hash returning 32-bit numbers.
 */
Napi::Value XxHash64BatchAsNumbers(const Napi::CallbackInfo& info) {
    Napi::Env env = info.Env();

    if (!info[0].IsArray()) {
        Napi::TypeError::New(env, "Expected Array of Buffers")
            .ThrowAsJavaScriptException();
        return env.Null();
    }

    auto buffers = info[0].As<Napi::Array>();
    uint32_t count = buffers.Length();

    uint64_t seed = 0;
    if (info.Length() > 1 && info[1].IsNumber()) {
        seed = static_cast<uint64_t>(info[1].As<Napi::Number>().Int64Value());
    }

    Napi::Array results = Napi::Array::New(env, count);

    for (uint32_t i = 0; i < count; i++) {
        Napi::Value item = buffers.Get(i);

        if (!item.IsBuffer() && !item.IsTypedArray()) {
            results.Set(i, Napi::Number::New(env, 0));
            continue;
        }

        uint8_t* data;
        size_t length;

        if (item.IsBuffer()) {
            auto buffer = item.As<Napi::Buffer<uint8_t>>();
            data = buffer.Data();
            length = buffer.Length();
        } else {
            auto typedArray = item.As<Napi::TypedArray>();
            data = static_cast<uint8_t*>(typedArray.ArrayBuffer().Data()) +
                   typedArray.ByteOffset();
            length = typedArray.ByteLength();
        }

        XXH64_hash_t hash = XXH64(data, length, seed);
        results.Set(i, Napi::Number::New(env, static_cast<uint32_t>(hash & 0xFFFFFFFF)));
    }

    return results;
}

/**
 * Create streaming hash state.
 * For hashing data incrementally.
 */
class XxHash64State : public Napi::ObjectWrap<XxHash64State> {
public:
    static Napi::Object Init(Napi::Env env, Napi::Object exports) {
        Napi::Function func = DefineClass(env, "XxHash64State", {
            InstanceMethod("update", &XxHash64State::Update),
            InstanceMethod("digest", &XxHash64State::Digest),
            InstanceMethod("digestAsNumber", &XxHash64State::DigestAsNumber),
            InstanceMethod("reset", &XxHash64State::Reset),
        });

        exports.Set("XxHash64State", func);
        return exports;
    }

    XxHash64State(const Napi::CallbackInfo& info)
        : Napi::ObjectWrap<XxHash64State>(info) {
        seed_ = 0;
        if (info.Length() > 0 && info[0].IsBigInt()) {
            bool lossless;
            seed_ = info[0].As<Napi::BigInt>().Uint64Value(&lossless);
        } else if (info.Length() > 0 && info[0].IsNumber()) {
            seed_ = static_cast<uint64_t>(info[0].As<Napi::Number>().Int64Value());
        }
        state_ = XXH64_createState();
        XXH64_reset(state_, seed_);
    }

    ~XxHash64State() {
        if (state_) {
            XXH64_freeState(state_);
        }
    }

    Napi::Value Update(const Napi::CallbackInfo& info) {
        Napi::Env env = info.Env();

        if (!info[0].IsBuffer() && !info[0].IsTypedArray()) {
            Napi::TypeError::New(env, "Expected Buffer or TypedArray")
                .ThrowAsJavaScriptException();
            return env.Undefined();
        }

        uint8_t* data;
        size_t length;

        if (info[0].IsBuffer()) {
            auto buffer = info[0].As<Napi::Buffer<uint8_t>>();
            data = buffer.Data();
            length = buffer.Length();
        } else {
            auto typedArray = info[0].As<Napi::TypedArray>();
            data = static_cast<uint8_t*>(typedArray.ArrayBuffer().Data()) +
                   typedArray.ByteOffset();
            length = typedArray.ByteLength();
        }

        XXH64_update(state_, data, length);
        return info.This();
    }

    Napi::Value Digest(const Napi::CallbackInfo& info) {
        Napi::Env env = info.Env();
        XXH64_hash_t hash = XXH64_digest(state_);
        return Napi::BigInt::New(env, hash);
    }

    Napi::Value DigestAsNumber(const Napi::CallbackInfo& info) {
        Napi::Env env = info.Env();
        XXH64_hash_t hash = XXH64_digest(state_);
        return Napi::Number::New(env, static_cast<uint32_t>(hash & 0xFFFFFFFF));
    }

    Napi::Value Reset(const Napi::CallbackInfo& info) {
        XXH64_reset(state_, seed_);
        return info.This();
    }

private:
    XXH64_state_t* state_;
    uint64_t seed_;
};

// Module initialization
Napi::Object Init(Napi::Env env, Napi::Object exports) {
    exports.Set("xxhash64", Napi::Function::New(env, XxHash64));
    exports.Set("xxhash64AsNumber", Napi::Function::New(env, XxHash64AsNumber));
    exports.Set("xxhash64Batch", Napi::Function::New(env, XxHash64Batch));
    exports.Set("xxhash64BatchAsNumbers", Napi::Function::New(env, XxHash64BatchAsNumbers));
    XxHash64State::Init(env, exports);
    return exports;
}

NODE_API_MODULE(topgun_hash, Init)
