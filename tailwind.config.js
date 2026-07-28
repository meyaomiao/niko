/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        gray: {
          50: "rgb(var(--nk-gray-50) / <alpha-value>)",
          100: "rgb(var(--nk-gray-100) / <alpha-value>)",
          200: "rgb(var(--nk-gray-200) / <alpha-value>)",
          300: "rgb(var(--nk-gray-300) / <alpha-value>)",
          400: "rgb(var(--nk-gray-400) / <alpha-value>)",
          500: "rgb(var(--nk-gray-500) / <alpha-value>)",
          600: "rgb(var(--nk-gray-600) / <alpha-value>)",
          700: "rgb(var(--nk-gray-700) / <alpha-value>)",
          800: "rgb(var(--nk-gray-800) / <alpha-value>)",
          900: "rgb(var(--nk-gray-900) / <alpha-value>)",
          950: "rgb(var(--nk-gray-950) / <alpha-value>)",
        },
        indigo: {
          300: "rgb(var(--nk-indigo-300) / <alpha-value>)",
          400: "rgb(var(--nk-indigo-400) / <alpha-value>)",
          500: "rgb(var(--nk-indigo-500) / <alpha-value>)",
          600: "rgb(var(--nk-indigo-600) / <alpha-value>)",
          700: "rgb(var(--nk-indigo-700) / <alpha-value>)",
        },
      },
      borderRadius: {
        lg: "0.75rem",
        xl: "1rem",
        "2xl": "1.25rem",
      },
      boxShadow: {
        sm: "var(--nk-shadow)",
      },
    },
  },
  plugins: [],
};
