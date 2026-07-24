package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
)

func main() {
	if err := run(os.Stdin, os.Stdout); err != nil {
		fmt.Fprintf(os.Stderr, "reference-go provider: %v\n", err)
		os.Exit(1)
	}
}

func run(input io.Reader, output io.Writer) error {
	const requestLimit = 4 * 1024 * 1024
	raw, err := io.ReadAll(io.LimitReader(input, requestLimit+1))
	if err != nil {
		return fmt.Errorf("read request: %w", err)
	}
	if len(raw) > requestLimit {
		return fmt.Errorf("request exceeds 4 MiB protocol limit")
	}
	if err := validateJSONDocument(raw); err != nil {
		return err
	}

	var request requestEnvelope
	if err := json.Unmarshal(raw, &request); err != nil {
		return fmt.Errorf("decode request: %w", err)
	}
	response, err := handleRequest(request)
	if err != nil {
		return err
	}
	encoded, err := json.Marshal(response)
	if err != nil {
		return fmt.Errorf("encode result: %w", err)
	}
	if len(encoded) > 1024*1024 {
		return fmt.Errorf("result exceeds 1 MiB protocol limit")
	}
	if _, err := output.Write(encoded); err != nil {
		return fmt.Errorf("write result: %w", err)
	}
	return nil
}

func validateJSONDocument(raw []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	token, err := decoder.Token()
	if err != nil {
		return fmt.Errorf("decode request: %w", err)
	}
	delimiter, ok := token.(json.Delim)
	if !ok || delimiter != '{' {
		return fmt.Errorf("request must be one JSON object")
	}
	if err := consumeComposite(decoder, delimiter); err != nil {
		return fmt.Errorf("decode request: %w", err)
	}
	if _, err := decoder.Token(); !errors.Is(err, io.EOF) {
		if err == nil {
			return fmt.Errorf("request contains trailing JSON value")
		}
		return fmt.Errorf("decode request trailing data: %w", err)
	}
	return nil
}

func consumeValue(decoder *json.Decoder) error {
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	if delimiter, ok := token.(json.Delim); ok {
		return consumeComposite(decoder, delimiter)
	}
	return nil
}

func consumeComposite(decoder *json.Decoder, opening json.Delim) error {
	switch opening {
	case '{':
		seen := map[string]struct{}{}
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return err
			}
			key, ok := token.(string)
			if !ok {
				return fmt.Errorf("object key is not a string")
			}
			if _, duplicate := seen[key]; duplicate {
				return fmt.Errorf("duplicate object key %q", key)
			}
			seen[key] = struct{}{}
			if err := consumeValue(decoder); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil {
			return err
		}
		if closing != json.Delim('}') {
			return fmt.Errorf("object missing closing delimiter")
		}
		return nil
	case '[':
		for decoder.More() {
			if err := consumeValue(decoder); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil {
			return err
		}
		if closing != json.Delim(']') {
			return fmt.Errorf("array missing closing delimiter")
		}
		return nil
	default:
		return fmt.Errorf("unexpected closing delimiter %q", opening)
	}
}
