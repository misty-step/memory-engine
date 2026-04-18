import type { RubricAssessment, RubricPrompt } from '../types';

export interface RubricGraderAdapter {
  grade(prompt: RubricPrompt, answer: string): Promise<RubricAssessment>;
}

export class StaticRubricGrader implements RubricGraderAdapter {
  readonly #assessment: RubricAssessment;

  constructor(assessment: RubricAssessment) {
    this.#assessment = assessment;
  }

  async grade(_prompt: RubricPrompt, _answer: string): Promise<RubricAssessment> {
    return this.#assessment;
  }
}
