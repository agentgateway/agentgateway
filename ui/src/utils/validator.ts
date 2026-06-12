import {
  customizeValidator,
  type CustomValidatorOptionsType,
} from "@rjsf/validator-ajv8";
import Ajv2020 from "ajv/dist/2020";

/**
 * Custom validator with support for JSON Schema draft 2020-12.
 *
 * The default @rjsf/validator-ajv8 uses Ajv with draft-07 support.
 * Since our schemas use draft 2020-12, we need to use Ajv2020.
 *
 * `validateFormats` is set to false because the generated schemas include
 * Rust-specific numeric format annotations (e.g. "uint16", "uint", "i32")
 * that Ajv does not recognise. These annotations carry no semantic validation
 * meaning for the UI — they are type hints for the Rust deserialiser — so
 * ignoring them is correct and eliminates the "unknown format … ignored"
 * console warnings.
 */
const customValidatorOptions: CustomValidatorOptionsType = {
  AjvClass: Ajv2020,
  ajvOptionsOverrides: {
    strict: false,
    allErrors: true,
    validateFormats: false,
    $data: true,
  },
};

export const validator = customizeValidator(customValidatorOptions);
