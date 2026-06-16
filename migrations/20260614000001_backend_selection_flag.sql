-- WS6 chunk 4a (§D): in-DB backend-selection gate.
-- A singleton config row in `public` choosing which substrate the surfaces
-- dispatch to. Default 'legacy' => install is zero behavior change. The flip
-- (chunk 5) is a trivial one-row UPDATE migration. Governs SURFACES, not
-- substrate, so it lives in `public`, not `temper_next`.

CREATE TABLE public.kb_backend_selection (
    id         boolean     PRIMARY KEY DEFAULT true,
    backend    text        NOT NULL DEFAULT 'legacy'
                           CHECK (backend IN ('legacy', 'next')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT kb_backend_selection_singleton CHECK (id = true)
);

INSERT INTO public.kb_backend_selection (id, backend) VALUES (true, 'legacy');
